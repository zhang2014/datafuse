// Copyright 2021 Datafuse Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fs::File;
use std::io::{BufRead, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
#[cfg(test)]
use std::ptr::addr_of_mut;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;
use backtrace::{Backtrace, BacktraceFrame};
use bincode::config::BigEndian;

use databend_common_base::runtime::{Thread, ThreadTracker};

use crate::panic_hook::captures_frames;

struct CrashHandler {
    version: String,
    write_file: File,
}

impl CrashHandler {
    pub fn create(version: String, write_file: File) -> CrashHandler {
        CrashHandler { version, write_file }
    }

    pub fn recv_signal(&mut self, sig: i32, info: *mut libc::siginfo_t, uc: *mut libc::c_void) {
        // let file = unsafe { File::from_raw_fd(self.write_file.as_raw_fd()) };

        let mut writer = std::io::BufWriter::new(&mut self.write_file);
        let current_query_id = match ThreadTracker::query_id() {
            None => "Unknown",
            Some(query_id) => query_id,
        };

        bincode::serde::encode_into_std_write(
            std::thread::current().id().as_u64(),
            &mut writer,
            bincode::config::standard(),
        ).unwrap();

        bincode::serde::encode_into_std_write(
            current_query_id,
            &mut writer,
            bincode::config::standard(),
        ).unwrap();
        // serde_json::to_writer(&mut writer, &std::thread::current().id().as_u64()).unwrap();
        // serde_json::to_writer(&mut writer, &current_query_id).unwrap();

        let mut frames = Vec::with_capacity(50);
        captures_frames(&mut frames);

        bincode::serde::encode_into_std_write(
            frames.len(),
            &mut writer,
            bincode::config::standard(),
        ).unwrap();
        // serde_json::to_writer(&mut writer, &frames.len()).unwrap();
        writer.flush().unwrap();

        for frame in frames {
            bincode::serde::encode_into_std_write(
                frame,
                &mut writer,
                bincode::config::standard(),
            ).unwrap();
            // serde_json::to_writer(&mut writer, &frame).unwrap();
        }

        // writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        std::thread::sleep(Duration::from_secs(4));
        // unsafe { libc::close(self.write_file.as_raw_fd()); }
        // write_error(format_args!("{:#^80}", " Crash fault info "));
        // write_error(format_args!("PID: {}", std::process::id()));
        // write_error(format_args!(
        //     "TID: {}",
        //     std::thread::current().id().as_u64()
        // ));
        // write_error(format_args!("Version: {}", self.version));
        // write_error(format_args!("Timestamp(UTC): {}", chrono::Utc::now()));
        // write_error(format_args!("Timestamp(Local): {}", chrono::Local::now()));
        // write_error(format_args!("QueryId: {:?}", current_query_id));
        // write_error(format_args!("{}", signal_message(sig, info, uc)));

        // write_error(format_args!("Backtrace:\n"));
        // for (idx, (name, file, location)) in captures_frames(50).into_iter().enumerate() {
        //     let has_hash_suffix = name.len() > 19
        //         && &name[name.len() - 19..name.len() - 16] == "::h"
        //         && name[name.len() - 16..]
        //         .chars()
        //         .all(|x| x.is_ascii_hexdigit());
        //
        //     match has_hash_suffix {
        //         true => write_error(format_args!("{:4}: {}", idx, &name[..name.len() - 19])),
        //         false => write_error(format_args!("{:4}: {}", idx, name)),
        //     }
        //
        //     write_error(format_args!("             at {}:{}", file, location));
        // }
    }
}

static CRASH_HANDLER_LOCK: Mutex<Option<CrashHandler>> = Mutex::new(None);

fn sigsegv_message(info: *mut libc::siginfo_t, _: *mut libc::c_void) -> String {
    unsafe {
        let mut address = String::from("null points");

        if !(*info).si_addr().is_null() {
            address = format!("{:#02x?}", (*info).si_addr() as usize);
        }

        format!(
            "Signal {} ({}), si_code {} ({}), Address {}\n",
            libc::SIGSEGV,
            "SIGSEGV",
            (*info).si_code,
            "Unknown", // TODO: SEGV_MAPERR or SEGV_ACCERR
            address,
        )
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
#[repr(C)]
pub struct __jmp_buf([u64; 8]);

#[cfg(all(test, target_arch = "arm"))]
#[repr(C)]
pub struct __jmp_buf([u64; 32]);

#[cfg(all(test, target_arch = "aarch64"))]
#[repr(C)]
pub struct __jmp_buf([u64; 22]);

#[cfg(test)]
#[repr(C)]
pub struct JmpBuffer {
    __jmp_buf: __jmp_buf,
    __fl: u32,
    __ss: [u32; 32],
}

#[cfg(test)]
impl JmpBuffer {
    pub const fn create() -> JmpBuffer {
        #[cfg(target_arch = "x86_64")]
        {
            JmpBuffer {
                __jmp_buf: crate::crash_hook::__jmp_buf([0; 8]),
                __fl: 0,
                __ss: [0; 32],
            }
        }

        #[cfg(target_arch = "arm")]
        {
            JmpBuffer {
                __jmp_buf: crate::crash_hook::__jmp_buf([0; 32]),
                __fl: 0,
                __ss: [0; 32],
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            JmpBuffer {
                __jmp_buf: crate::crash_hook::__jmp_buf([0; 22]),
                __fl: 0,
                __ss: [0; 32],
            }
        }
    }
}

#[cfg(test)]
extern "C" {
    // https://man7.org/linux/man-pages/man3/sigsetjmp.3p.html
    #[cfg_attr(target_env = "gnu", link_name = "__sigsetjmp")]
    pub fn sigsetjmp(jb: *mut JmpBuffer, save_mask: i32) -> i32;

    // https://man7.org/linux/man-pages/man3/siglongjmp.3p.html
    pub fn siglongjmp(jb: *mut JmpBuffer, val: i32) -> !;
}

#[cfg(test)]
static mut TEST_JMP_BUFFER: JmpBuffer = JmpBuffer::create();

fn sigbus_message(info: *mut libc::siginfo_t, _: *mut libc::c_void) -> String {
    unsafe {
        format!(
            "Signal {} ({}), si_code {} ({})\n",
            libc::SIGBUS,
            "SIGBUS",
            (*info).si_code,
            match (*info).si_code {
                libc::BUS_ADRALN => "BUS_ADRALN, invalid address alignment",
                libc::BUS_ADRERR => "BUS_ADRERR, non-existent physical address",
                libc::BUS_OBJERR => "BUS_OBJERR, object specific hardware error",
                _ => "Unknown",
            },
        )
    }
}

fn sigill_message(info: *mut libc::siginfo_t, _: *mut libc::c_void) -> String {
    unsafe {
        format!(
            "Signal {} ({}), si_code {} ({})， instruction address:{} \n",
            libc::SIGILL,
            "SIGILL",
            (*info).si_code,
            "Unknown", /* ILL_ILLOPC ILL_ILLOPN ILL_ILLADR ILL_ILLTRP ILL_PRVOPC ILL_PRVREG ILL_COPROC ILL_BADSTK, */
            match (*info).si_addr().is_null() {
                true => "null points".to_string(),
                false => format!("{:#02x?}", (*info).si_addr() as usize),
            },
        )
    }
}

fn sigfpe_message(info: *mut libc::siginfo_t, _: *mut libc::c_void) -> String {
    unsafe {
        format!(
            "Signal {} ({}), si_code {} ({})， instruction address:{} \n",
            libc::SIGFPE,
            "SIGFPE",
            (*info).si_code,
            "Unknown", /* FPE_INTDIV FPE_INTOVF FPE_FLTDIV FPE_FLTOVF FPE_FLTUND FPE_FLTRES FPE_FLTINV FPE_FLTSUB */
            match (*info).si_addr().is_null() {
                true => "null points".to_string(),
                false => format!("{:#02x?}", (*info).si_addr() as usize),
            },
        )
    }
}

fn signal_message(sig: i32, info: *mut libc::siginfo_t, uc: *mut libc::c_void) -> String {
    // https://pubs.opengroup.org/onlinepubs/007908799/xsh/signal.h.html
    match sig {
        libc::SIGBUS => sigbus_message(info, uc),
        libc::SIGILL => sigill_message(info, uc),
        libc::SIGSEGV => sigsegv_message(info, uc),
        libc::SIGFPE => sigfpe_message(info, uc),
        _ => format!("Signal {}, si_code {}", sig, unsafe { (*info).si_code }),
    }
}

#[cfg(test)]
static mut ERROR_MESSAGE: String = String::new();

fn write_error(message: std::fmt::Arguments) {
    #[cfg(test)]
    unsafe {
        ERROR_MESSAGE.push_str(&format!("{}\n", message))
    };

    #[cfg(not(test))]
    eprintln!("{}", message);
}

unsafe extern "C" fn signal_handler(sig: i32, info: *mut libc::siginfo_t, uc: *mut libc::c_void) {
    let lock = CRASH_HANDLER_LOCK.lock();
    let mut guard = lock.unwrap_or_else(PoisonError::into_inner);

    if let Some(crash_handler) = guard.as_mut() {
        crash_handler.recv_signal(sig, info, uc);
    }

    #[cfg(test)]
    {
        drop(guard);
        siglongjmp(addr_of_mut!(TEST_JMP_BUFFER), 1);
    }

    #[allow(unreachable_code)]
    if sig != libc::SIGTRAP {
        drop(guard);
        let _ = std::io::stderr().flush();
        std::process::exit(1);
    }
}

pub unsafe fn add_signal_handler(signals: Vec<i32>) {
    let mut sa = std::mem::zeroed::<libc::sigaction>();

    sa.sa_flags = libc::SA_ONSTACK | libc::SA_SIGINFO;
    sa.sa_sigaction = signal_handler as usize;

    libc::sigemptyset(&mut sa.sa_mask);

    for signal in &signals {
        libc::sigaddset(&mut sa.sa_mask, *signal);
    }

    for signal in &signals {
        libc::sigaction(*signal, &sa, std::ptr::null_mut());
    }
}

// https://man7.org/linux/man-pages/man2/sigaltstack.2.html
pub unsafe fn add_signal_stack(stack_bytes: usize) {
    let page_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
    let alloc_size = page_size + stack_bytes;

    let stack_memory_arena = libc::mmap(
        std::ptr::null_mut(),
        alloc_size,
        libc::PROT_NONE,
        libc::MAP_PRIVATE | libc::MAP_ANON,
        -1,
        0,
    );

    if stack_memory_arena == libc::MAP_FAILED {
        return;
    }

    let stack_ptr = (stack_memory_arena as usize + page_size) as *mut libc::c_void;

    if libc::mprotect(stack_ptr, stack_bytes, libc::PROT_READ | libc::PROT_WRITE) != 0 {
        libc::munmap(stack_ptr, alloc_size);
        return;
    }

    let mut new_signal_stack = std::mem::zeroed::<libc::stack_t>();
    new_signal_stack.ss_sp = stack_memory_arena;
    new_signal_stack.ss_size = stack_bytes;
    if libc::sigaltstack(&new_signal_stack, std::ptr::null_mut()) != 0 {
        libc::munmap(stack_ptr, alloc_size);
    }
}

pub fn set_crash_hook(version: String) {
    let lock = CRASH_HANDLER_LOCK.lock();
    let mut guard = lock.unwrap_or_else(PoisonError::into_inner);

    let (infile, outfile) = pipe_file().unwrap();

    *guard = Some(CrashHandler::create(version, outfile));
    unsafe {
        #[cfg(debug_assertions)]
        add_signal_stack(20 * 1024 * 1024);

        #[cfg(not(debug_assertions))]
        add_signal_stack(2 * 1024 * 1024);

        add_signal_handler(vec![
            libc::SIGSEGV,
            libc::SIGILL,
            libc::SIGBUS,
            libc::SIGFPE,
            libc::SIGSYS,
            libc::SIGTRAP,
        ]);
    };


    SignalListener::spawn(infile);
}

#[cfg(not(target_os = "macos"))]
fn open_pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    unsafe {
        let mut fds: [libc::c_int; 2] = [0; 2];
        if libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])))
    }
}

#[cfg(target_os = "macos")]
fn open_pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    unsafe {
        let mut fds: [libc::c_int; 2] = [0; 2];

        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Err(std::io::Error::last_os_error());
        }

        if libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC) != 0 {
            return Err(std::io::Error::last_os_error());
        }

        if libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC) != 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])))
    }
}

fn pipe_file() -> std::io::Result<(File, File)> {
    let (ifd, ofd) = open_pipe()?;

    unsafe { Ok((File::from_raw_fd(ifd.into_raw_fd()), File::from_raw_fd(ofd.into_raw_fd()))) }
}

struct SignalListener;

impl SignalListener {
    pub fn spawn(file: File) {
        Thread::named_spawn(Some(String::from("SignalListener")), move || {
            let mut reader = std::io::BufReader::new(file);
            while let Ok(true) = reader.has_data_left() {
                let tid: u64 = bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap();
                let query_id: String = bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap();
                let frame_size: usize = bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap();

                let mut frames = Vec::<BacktraceFrame>::with_capacity(frame_size);
                for index in 0..frame_size {
                    frames.push(bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap());
                }

                let mut backtrace = Backtrace::from(frames);
                backtrace.resolve();
                eprintln!("{:?}", backtrace);
                log::info!("{:?}", backtrace);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Read};
    use std::ptr::addr_of_mut;
    use backtrace::{Backtrace, BacktraceFrame};

    use databend_common_base::runtime::ThreadTracker;

    use crate::crash_hook::{open_pipe, pipe_file, sigsetjmp};
    use crate::crash_hook::ERROR_MESSAGE;
    use crate::crash_hook::TEST_JMP_BUFFER;
    use crate::set_crash_hook;

    #[test]
    fn test_crash() {
        unsafe {
            let (input, output) = pipe_file().unwrap();
            set_crash_hook(String::from("1.2.111"), output);
            let mut reader = std::io::BufReader::new(input);

            for signal in [
                libc::SIGSEGV,
                // libc::SIGILL,
                // libc::SIGBUS,
                // libc::SIGFPE,
                // libc::SIGSYS,
                // libc::SIGTRAP,
            ] {
                // ERROR_MESSAGE = String::new();
                let query_id = format!("Trakcing query id: {}", signal);
                let mut tracking_payload = ThreadTracker::new_tracking_payload();
                tracking_payload.query_id = Some(query_id.clone());

                let _guard = ThreadTracker::tracking(tracking_payload);

                if sigsetjmp(addr_of_mut!(TEST_JMP_BUFFER), 1) == 0 {
                    libc::raise(signal);
                }

                // Example:
                // ############################### Crash fault info ###############################
                // PID: 0
                // Version: 1.2.111
                // Timestamp(UTC): 2024-07-15 07:24:50.624669 UTC
                // Timestamp(Local): 2024-07-15 15:24:50.624788 +08:00
                // QueryId: "Trakcing query id: 11"
                // Signal 11 (SIGSEGV), si_code 2 (Unknown), Address 0x10479d130
                //
                // Backtrace:
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ BACKTRACE ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                //                               ⋮ 5 frames hidden ⋮
                //  6: databend_common_tracing::panic_hook::backtrace::h205501a90720b8ac
                //     at /Users/WinterZhang/Source/databend/src/common/tracing/src/panic_hook.rs:67
                //       65 │ pub fn backtrace() -> String {
                //       66 │     if should_backtrace() {
                //       67 >         let backtrace = Backtrace::new();
                //       68 │         let printer = BacktracePrinter::new()
                //       69 │             .message("")
                //  7: databend_common_tracing::crash_hook::CrashHandler::recv_signal::hfbc5155e9fe371e2
                //     at /Users/WinterZhang/Source/databend/src/common/tracing/src/crash_hook.rs:31
                //       29 │             write_error(format_args!("QueryId: {:?}", current_query_id));
                //       30 │             write_error(format_args!("{}", signal_message(sig, info, uc)));
                //       31 >             write_error(format_args!("Backtrace:\n{}", backtrace()));
                //       32 │         }
                //       33 │     }
                //  8: databend_common_tracing::crash_hook::signal_handler::h8f436acfc653eadf
                //     at /Users/WinterZhang/Source/databend/src/common/tracing/src/crash_hook.rs:202
                //      200 │
                //      201 │     if let Some(crash_handler) = guard.as_ref() {
                //      202 >         crash_handler.recv_signal(sig, info, uc);
                //      203 │     }
                //      204 │
                //  9: _OSAtomicTestAndClearBarrier
                //     at <unknown source file>
                // 10: __pthread_atfork_prepare_handlers
                //     at <unknown source file>
                // 11: databend_common_tracing::crash_hook::tests::test_crash::hb571a3876f0deaa5
                //     at /Users/WinterZhang/Source/databend/src/common/tracing/src/crash_hook.rs:274
                //      272 │
                //      273 │                 if sigsetjmp(addr_of_mut!(TEST_JMP_BUFFER), 1) == 0 {
                //      274 >                     libc::raise(signal);
                //      275 │                 }
                //      276 │
                // 12: databend_common_tracing::crash_hook::tests::test_crash::{{closure}}::h85bf763847dcff53
                //     at /Users/WinterZhang/Source/databend/src/common/tracing/src/crash_hook.rs:261
                //      259 │
                //      260 │     #[test]
                //      261 >     fn test_crash() {
                //      262 │         unsafe {
                //      263 │             set_crash_hook(String::from("1.2.111"));
                // 14: core::ops::function::FnOnce::call_once::h31eb0fafb294dd12
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/core/src/ops/function.rs:250
                // 15: test::__rust_begin_short_backtrace::h6fac42d75080a771
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/test/src/lib.rs:620
                // 16: test::run_test_in_process::{{closure}}::h55d5037f9addec80
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/test/src/lib.rs:643
                // 17: <core::panic::unwind_safe::AssertUnwindSafe<F> as core::ops::function::FnOnce<()>>::call_once::hdb2a8a04750edb82
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/core/src/panic/unwind_safe.rs:272
                // 18: std::panicking::try::do_call::h6d5a55026eb7cc8c
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/std/src/panicking.rs:554
                // 19: std::panicking::try::h64d2a41424d099bc
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/std/src/panicking.rs:518
                // 20: std::panic::catch_unwind::hc4df77f0513e1be7
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/std/src/panic.rs:142
                // 21: test::run_test_in_process::ha75b98814770ae24
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/test/src/lib.rs:643
                // 22: test::run_test::{{closure}}::h7f301602b56ea75e
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/test/src/lib.rs:566
                // 23: test::run_test::{{closure}}::hfbc0ddafa8073094
                //     at /rustc/8ace7ea1f7cbba7b4f031e66c54ca237a0d65de6/library/test/src/lib.rs:594
                //                               ⋮ 12 frames hidden ⋮

                // assert!(!ERROR_MESSAGE.is_empty());
                // assert!(ERROR_MESSAGE.contains("1.2.111"));
                // assert!(ERROR_MESSAGE.contains(&query_id));
                // assert!(ERROR_MESSAGE.contains(&format!("Signal {}", signal)));
                // assert!(ERROR_MESSAGE.contains("Backtrace"));
                // assert!(ERROR_MESSAGE.contains("test_crash"));
                // input.read_to_end()

                let tid: u64 = bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap();
                let query_id: String = bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap();
                let frame_size: usize = bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap();

                let mut frames = Vec::<BacktraceFrame>::with_capacity(frame_size);
                for index in 0..frame_size {
                    frames.push(bincode::serde::decode_from_reader(&mut reader, bincode::config::standard()).unwrap());
                }
                //
                let mut backtrace = Backtrace::from(frames);
                backtrace.resolve();
                eprintln!("{:?}", backtrace);
                // let mut ss = String::new();
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", reader.read_line(&mut ss));
                // println!("{:?}", ss);
                // input.read_to_string(&mut ss).unwrap();
                // eprintln!("{}", ss);
            }
        }
    }
}

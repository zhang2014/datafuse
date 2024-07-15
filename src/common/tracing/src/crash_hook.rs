use std::{mem, ptr};
use std::sync::Mutex;
use std::sync::PoisonError;
use crash_handler::{CrashContext, CrashEvent, CrashEventResult};

use crate::panic_hook::backtrace;

static CRASH_HANDLER_LOCK: Mutex<Option<crash_handler::CrashHandler>> = Mutex::new(None);

static mut VERSION: String = String::new();
static mut SIGNALS: [libc::sighandler_t; 32] = [0; 32];

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
        _ => String::new(),
    }
}

unsafe fn write_error(message: impl Into<String>) {
    let message = message.into();
    libc::write(2, message.as_ptr().cast(), message.len());
}

unsafe extern "C" fn signal_handler(sig: i32, info: *mut libc::siginfo_t, uc: *mut libc::c_void) {

    {
        let mut cur_handler = mem::zeroed();
        if libc::sigaction(sig as i32, ptr::null_mut(), &mut cur_handler) == 0
            && cur_handler.sa_sigaction == signal_handler as usize
            && cur_handler.sa_flags & libc::SA_SIGINFO == 0
        {
            // Reset signal handler with the correct flags.
            libc::sigemptyset(&mut cur_handler.sa_mask);
            libc::sigaddset(&mut cur_handler.sa_mask, sig as i32);

            cur_handler.sa_sigaction = signal_handler as usize;
            cur_handler.sa_flags = libc::SA_ONSTACK | libc::SA_SIGINFO;

            if libc::sigaction(sig as i32, &cur_handler, ptr::null_mut()) == -1 {
                // When resetting the handler fails, try to reset the
                // default one to avoid an infinite loop here.
                install_default_handler(sig);
            }

            // exit the handler as we should be called again soon
            return;
        }
    }

    // write_error(format!("{:#^80}\n", " Crash fault info "));
    // write_error(format!("PID: {}\n", (*info).si_pid()));
    // write_error(format!("Version: {}\n", VERSION));
    // write_error(format!("Timestamp(UTC): {}\n", chrono::Utc::now()));
    // write_error(signal_message(sig, info, uc));
    // write_error("\nBacktrace:\n");
    write_error(backtrace());

    std::process::exit(1);
}

pub unsafe fn add_signal_handler(signals: Vec<i32>) {
    for signal in signals {
        if signal < 32 && SIGNALS[signal as usize] == 0 {
            let mut sa = std::mem::zeroed::<libc::sigaction>();
            libc::sigaction(signal, std::ptr::null(), &mut sa);
            SIGNALS[signal as usize] = sa.sa_sigaction;
            sa.sa_sigaction = signal_handler as usize;
            libc::sigaction(signal, &sa, std::ptr::null_mut());
        }
    }
}


struct CrashHandler;

unsafe impl CrashEvent for CrashHandler {
    fn on_crash(&self, context: &CrashContext) -> CrashEventResult {
        unsafe { write_error(backtrace()); }
        CrashEventResult::Handled(false)
    }
}

const fn get_stack_size() -> usize {
    if libc::SIGSTKSZ > 16 * 1024 {
        libc::SIGSTKSZ
    } else {
        16 * 1024
    }
}

const SIG_STACK_SIZE: usize = get_stack_size();

pub unsafe fn install_sigaltstack() {
    // Check to see if the existing sigaltstack, and if it exists, is it big
    // enough. If so we don't need to allocate our own.
    let mut old_stack = mem::zeroed();
    let r = libc::sigaltstack(ptr::null(), &mut old_stack);
    assert_eq!(
        r,
        0,
        "learning about sigaltstack failed: {}",
        std::io::Error::last_os_error()
    );

    if old_stack.ss_flags & libc::SS_DISABLE == 0 && old_stack.ss_size >= SIG_STACK_SIZE {
        return;
    }

    // ... but failing that we need to allocate our own, so do all that
    // here.
    let guard_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
    let alloc_size = guard_size + SIG_STACK_SIZE;

    let ptr = libc::mmap(
        ptr::null_mut(),
        alloc_size,
        libc::PROT_NONE,
        libc::MAP_PRIVATE | libc::MAP_ANON,
        -1,
        0,
    );

    // Prepare the stack with readable/writable memory and then register it
    // with `sigaltstack`.
    let stack_ptr = (ptr as usize + guard_size) as *mut libc::c_void;
    let r = libc::mprotect(
        stack_ptr,
        SIG_STACK_SIZE,
        libc::PROT_READ | libc::PROT_WRITE,
    );
    assert_eq!(
        r,
        0,
        "mprotect to configure memory for sigaltstack failed: {}",
        std::io::Error::last_os_error()
    );
    let new_stack = libc::stack_t {
        ss_sp: stack_ptr,
        ss_flags: 0,
        ss_size: SIG_STACK_SIZE,
    };
    let r = libc::sigaltstack(&new_stack, ptr::null_mut());
    assert_eq!(
        r,
        0,
        "registering new sigaltstack failed: {}",
        std::io::Error::last_os_error()
    );

    // *STACK_SAVE.lock() = Some(StackSave {
    //     old: (old_stack.ss_flags & libc::SS_DISABLE != 0).then_some(old_stack),
    //     new: new_stack,
    // });

    // Ok(())
}

pub fn set_crash_hook(version: String) {
    let mut guard = CRASH_HANDLER_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    *guard = Some(crash_handler::CrashHandler::attach(Box::new(CrashHandler)).unwrap());
    // unsafe {
    //     VERSION = version;
    //     install_sigaltstack();
    //     add_signal_handler(vec![
    //         libc::SIGSEGV,
    //         libc::SIGILL,
    //         libc::SIGBUS,
    //         libc::SIGFPE,
    //         libc::SIGSYS,
    //     ]);
    // }
}

#[cfg(test)]
mod tests {
    use crate::set_crash_hook;

    #[test]
    fn test_crash() {
        set_crash_hook(String::from("1.2.111"));

        sigsegv_fun();
    }

    #[allow(unused)]
    fn sigsegv_fun() {
        unsafe { std::ptr::null_mut::<i32>().write(42) };
    }
}

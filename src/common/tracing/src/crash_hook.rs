use std::backtrace::Backtrace;
use std::mem;
use std::sync::{LazyLock, Mutex, PoisonError};
use crash_handler::{CrashContext, CrashEvent, CrashEventResult, CrashHandler, Error};

struct CrashHook;

unsafe impl CrashEvent for CrashHook {
    fn on_crash(&self, context: &CrashContext) -> CrashEventResult {
        log::error!("{:?}", Backtrace::force_capture());
        eprintln!("{:?}", Backtrace::force_capture());
        // std::backtrace::Backtrace::capture()
        // context
        // context.handler_thread
        // if let Some(exception) = context.exception {
        //     exception.code
        // }
        // context
        CrashEventResult::Handled(false)
    }
}

static HANDLER: Mutex<Option<CrashHandler>> = Mutex::new(None);

// fn add_signal_handler(signal_function:)

unsafe extern "C" fn signal_handler(sig: i32, info: *mut libc::siginfo_t, uc: *mut libc::c_void) {
    eprintln!("handled {}", sig);
}

pub unsafe fn add_signal_handler(signals: Vec<i32>) {
    let mut sa = std::mem::zeroed::<libc::sigaction>();
    sa.sa_sigaction = signal_handler as usize;
    sa.sa_flags = libc::SA_SIGINFO;

    libc::sigemptyset(&mut sa.sa_mask);

    for signal in &signals {
        libc::sigaddset(&mut sa.sa_mask, *signal);
    }

    for signal in &signals {
        libc::sigaction(*signal, &sa, std::ptr::null_mut());
    }
}

pub fn set_crash_hook() {
    unsafe { add_signal_handler(vec![libc::SIGABRT, libc::SIGSEGV]) }
}

#[cfg(test)]
mod tests {
    use crate::set_crash_hook;

    #[test]
    fn test_crash() {
        set_crash_hook();

        unsafe { std::ptr::null_mut::<i32>().write(42) };
    }
}

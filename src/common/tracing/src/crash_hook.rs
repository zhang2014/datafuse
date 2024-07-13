use std::sync::Mutex;
use std::sync::PoisonError;

use crate::panic_hook::backtrace;

static CRASH_HANDLER_LOCK: Mutex<()> = Mutex::new(());

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
    write_error(format!("{:#^80}\n", " Crash fault info "));
    write_error(format!("PID: {}\n", (*info).si_pid()));
    write_error(format!("Version: {}\n", VERSION));
    write_error(format!("Timestamp(UTC): {}\n", chrono::Utc::now()));
    write_error(signal_message(sig, info, uc));
    write_error("\nBacktrace:\n");
    write_error(backtrace());

    if sig < 32 && SIGNALS[sig as usize] != 0 {
        let fn2: extern "C" fn(i32, *mut libc::siginfo_t, *mut libc::c_void) =
            std::mem::transmute(SIGNALS[sig as usize]);

        fn2(sig, info, uc);
    }
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

pub fn set_crash_hook(version: String) {
    let _guard = CRASH_HANDLER_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    unsafe {
        VERSION = version;
        add_signal_handler(vec![
            libc::SIGSEGV,
            libc::SIGILL,
            libc::SIGBUS,
            libc::SIGFPE,
            libc::SIGSYS,
        ]);
    }
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

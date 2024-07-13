use std::backtrace::Backtrace;
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
        todo!()
    }
}

static HANDLER: Mutex<Option<CrashHandler>> = Mutex::new(None);

pub fn set_crash_hook() {
    match CrashHandler::attach(Box::new(CrashHook)) {
        Ok(handler) => {
            let mut guard = HANDLER.lock().unwrap_or_else(PoisonError::into_inner);
            *guard = Some(handler);
        }
        Err(cause) => {
            log::error!("Attach crash handler failure, cause: {:?}", cause);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::set_crash_hook;

    #[test]
    fn test_crash() {
        set_crash_hook();

        struct SS;

        unsafe { std::ptr::null_mut::<i32>().write(42) };
    }
}

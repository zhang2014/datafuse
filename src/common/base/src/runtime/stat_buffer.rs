use std::sync::atomic::Ordering;

use databend_common_exception::Result;

use crate::runtime::runtime_tracker::OutOfLimit;
use crate::runtime::LimitMemGuard;
use crate::runtime::MemStat;
use crate::runtime::ThreadTracker;

static MEM_STAT_BUFFER_SIZE: i64 = 4 * 1024 * 1024;

/// Buffering memory allocation stats.
///
/// A StatBuffer buffers stats changes in local variables, and periodically flush them to other storage such as an `Arc<T>` shared by several threads.
#[derive(Clone)]
pub struct StatBuffer {
    memory_usage: i64,
    // Whether to allow unlimited memory. Alloc memory will not panic if it is true.
    unlimited_flag: bool,
    global_mem_stat: &'static MemStat,
    destroyed_thread_local_macro: bool,
}

impl StatBuffer {
    pub const fn empty(global_mem_stat: &'static MemStat) -> Self {
        Self {
            memory_usage: 0,
            global_mem_stat,
            unlimited_flag: false,
            destroyed_thread_local_macro: false,
        }
    }

    pub fn is_unlimited(&self) -> bool {
        self.unlimited_flag
    }

    pub fn set_unlimited_flag(&mut self, flag: bool) -> bool {
        let old = self.unlimited_flag;
        self.unlimited_flag = flag;
        old
    }

    pub fn incr(&mut self, bs: i64) -> i64 {
        self.memory_usage += bs;
        self.memory_usage
    }

    /// Flush buffered stat to MemStat it belongs to.
    pub fn flush<const NEED_ROLLBACK: bool>(&mut self) -> Result<(), OutOfLimit> {
        match std::mem::take(&mut self.memory_usage) {
            0 => Ok(()),
            memory_usage => ThreadTracker::record_memory::<NEED_ROLLBACK>(memory_usage),
        }
    }

    pub fn alloc(&mut self, memory_usage: i64) -> Result<(), OutOfLimit> {
        // Rust will alloc or dealloc memory after the thread local is destroyed when we using thread_local macro.
        // This is the boundary of thread exit. It may be dangerous to throw mistakes here.
        if self.destroyed_thread_local_macro {
            let used = self
                .global_mem_stat
                .used
                .fetch_add(memory_usage, Ordering::Relaxed);
            self.global_mem_stat
                .peak_used
                .fetch_max(used + memory_usage, Ordering::Relaxed);
            return Ok(());
        }

        match self.incr(memory_usage) <= MEM_STAT_BUFFER_SIZE {
            true => Ok(()),
            false => self.flush::<true>(),
        }
    }

    pub fn dealloc(&mut self, memory_usage: i64) {
        // Rust will alloc or dealloc memory after the thread local is destroyed when we using thread_local macro.
        if self.destroyed_thread_local_macro {
            self.global_mem_stat
                .used
                .fetch_add(-memory_usage, Ordering::Relaxed);
            return;
        }

        if self.incr(-memory_usage) < -MEM_STAT_BUFFER_SIZE {
            let _ = self.flush::<false>();
        }

        // NOTE: De-allocation does not panic
        // even when it's possible exceeding the limit
        // due to other threads sharing the same MemStat may have allocated a lot of memory.
    }

    pub fn mark_destroyed(&mut self) {
        let _guard = LimitMemGuard::enter_unlimited();
        let memory_usage = std::mem::take(&mut self.memory_usage);

        // Memory operations during destruction will be recorded to global stat.
        self.destroyed_thread_local_macro = true;
        let _ = self.global_mem_stat.record_memory::<false>(memory_usage);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use databend_common_exception::Result;

    use crate::runtime::stat_buffer::StatBuffer;
    use crate::runtime::MemStat;

    #[test]
    fn test_mark_destroyed() -> Result<()> {
        static TEST_MEM_STATE: MemStat = MemStat::global();

        let mut buffer = StatBuffer::empty(&TEST_MEM_STATE);

        assert_eq!(buffer.destroyed_thread_local_macro, false);
        buffer.alloc(1).unwrap();
        assert_eq!(TEST_MEM_STATE.used.load(Ordering::Relaxed), 0);
        buffer.mark_destroyed();
        assert_eq!(buffer.destroyed_thread_local_macro, true);
        assert_eq!(TEST_MEM_STATE.used.load(Ordering::Relaxed), 1);
        buffer.alloc(1).unwrap();
        assert_eq!(TEST_MEM_STATE.used.load(Ordering::Relaxed), 2);

        Ok(())
    }
}

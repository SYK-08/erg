#[cfg(all(unix, feature = "debug"))]
pub use backtrace_on_stack_overflow;
use std::thread;

#[macro_export]
macro_rules! enable_overflow_stacktrace {
    () => {
        #[cfg(all(unix, feature = "debug"))]
        unsafe {
            $crate::spawn::backtrace_on_stack_overflow::enable()
        };
    };
}

/// Execute a function in a new thread on Windows, otherwise just run it.
///
/// Windows has a smaller default stack size than other OSs, which may cause a stack overflow, especially in the parsing process.
pub fn exec_new_thread<F, T>(run: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    enable_overflow_stacktrace!();
    if cfg!(windows) || cfg!(feature = "large_thread") {
        const STACK_SIZE: usize = 4 * 1024 * 1024;
        let child = thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(run)
            .unwrap();
        // Wait for thread to join
        child.join().unwrap()
    } else {
        run()
    }
}

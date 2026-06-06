//! Per-job debug logging context.
//!
//! Provides `debug_log!` — like `eprintln!` but prepends `[job:id]`
//! when a job context is active.  The main loop sets/clears the prefix
//! via [`set`] / [`clear`].

use std::sync::Mutex;

/// Set by the main loop to tag every debug line with the current job.
/// Format: `[job:id]`.  `pub(crate)` because the `debug_log!` macro in
/// the parent module accesses it via `use log_ctx::JOB_PREFIX;`.
pub(crate) static JOB_PREFIX: Mutex<String> = Mutex::new(String::new());

/// Enable job context.  Subsequent `debug_log!` calls prepend `[job:{id}]`.
pub fn set(id: &str) {
    *JOB_PREFIX.lock().unwrap() = format!("[job:{}]", id);
}

/// Clear job context.  Subsequent `debug_log!` calls print without prefix.
pub fn clear() {
    *JOB_PREFIX.lock().unwrap() = String::new();
}

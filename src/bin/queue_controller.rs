//! Queue controller entrypoint.
//!
//! Module tree lives in `src/queue_controller/`.
//! This file is just the thin binary wrapper.

#[path = "../queue_controller/mod.rs"]
mod app;

fn main() {
    match app::run() {
        Ok(()) => eprintln!("[DEBUG] Queue controller finished successfully"),
        Err(e) => {
            eprintln!("[ERROR] {e:#}");
            std::process::exit(1);
        }
    }
}

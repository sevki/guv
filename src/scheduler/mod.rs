//! Preemptive scheduler for stackless guests on Linux.
//!
//! The architecture modules provide the guest code and the register
//! save/restore logic; everything else is shared.

#[cfg_attr(target_arch = "aarch64", path = "aarch64.rs")]
#[cfg_attr(target_arch = "x86_64", path = "x86_64.rs")]
mod arch;
mod timer;
mod worker;

use arch::{Task, cruncher};
use worker::Worker;

const GUEST_COUNT: usize = 5;

pub fn run() -> ! {
    let worker = Worker::new((0..GUEST_COUNT).map(|_| Task::new(cruncher)).collect());
    if let Err(error) = unsafe { worker.start_timer() } {
        eprintln!("failed to start scheduler: {error}");
        std::process::exit(1);
    }
    match worker.run() {
        Ok(status) => std::process::exit(status as i32),
        Err(error) => {
            eprintln!("failed to run scheduler: {error}");
            std::process::exit(1);
        }
    }
}

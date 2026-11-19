mod round_robin;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
mod scheduler;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
fn main() {
    scheduler::run()
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "aarch64", target_arch = "x86_64")
)))]
fn main() {
    eprintln!("guv requires Linux on AArch64 or x86_64");
    std::process::exit(2);
}

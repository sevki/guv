# guv

`guv` is a small preemptive userland scheduler for Linux on AArch64. It runs
five stackless number-crunching guests on one thread and switches between them
on a 2 ms timer. The context switcher is specialized for these guests and does
not provide guest stacks.

Run it with `cargo run --release` on an AArch64 Linux host. Other targets build
for development and testing but report that they are unsupported at runtime.
The Linux/AArch64 implementation also supports musl; when cross-compiling,
configure an AArch64 musl linker and use
`cargo run --release --target aarch64-unknown-linux-musl`.

# guv

`guv` is a small preemptive userland scheduler for Linux on AArch64 and x86_64.
It runs five stackless number-crunching guests on one thread and switches
between them on a 2 ms timer. The context switchers are specialized for these
guests and do not provide guest stacks.

Run it with `cargo run --release` on either supported Linux architecture. Other
targets build for development and testing but report that they are unsupported
at runtime. Both supported architectures also support musl; when cross-
compiling, configure a linker for the target and use the appropriate
`aarch64-unknown-linux-musl` or `x86_64-unknown-linux-musl` target.

The design is documented in an [mdBook](book/src/SUMMARY.md); build it locally
with `mdbook serve book`.

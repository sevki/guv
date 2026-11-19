# Running guv

On a supported Linux machine:

```sh
cargo run --release
```

The process exits with status 0 once all five guests have finished. A
non-zero status means a guest computed a wrong result (AArch64), or that the
timer or signal handler could not be set up; in the second case an error is
printed to stderr.

## musl

Add the target and run it:

```sh
rustup target add aarch64-unknown-linux-musl   # or x86_64-unknown-linux-musl
cargo run --release --target aarch64-unknown-linux-musl
```

To cross-compile for the other architecture, configure a linker for that
target in `.cargo/config.toml` or with the matching
`CARGO_TARGET_<TRIPLE>_LINKER` environment variable.

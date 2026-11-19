# Testing

```sh
cargo test --all-targets
```

The tests are:

- `round_robin::tests`: unit tests for `next_runnable`, run on every
  platform.
- `scheduler::worker::tests::timer_preempts_and_completes_each_guest`: runs
  the real scheduler with five guests and checks that it exits with status 0,
  that the timer caused at least one switch, and that every guest finished.
  It saves and restores the process's `SIGALRM` action, alternate stack and
  signal mask so other tests are unaffected.

## Continuous integration

`.github/workflows/ci.yml` runs `cargo fmt --check` and the tests for the
glibc and musl targets on native x86_64 and AArch64 runners.

`.github/workflows/book.yml` builds this book on every push and pull request,
and deploys it to GitHub Pages from `main`.

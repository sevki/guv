# Introduction

`guv` is a small preemptive scheduler that runs entirely in user space. A
single thread runs five *guest* tasks. Every 2 ms a POSIX timer interrupts
whichever guest is running, and the signal handler swaps the next guest's
registers into the interrupted context. When the handler returns, the kernel
resumes the other guest.

The guests are deliberately simple: number-crunching loops written in
assembly that never touch the stack, call functions or make system calls.
That restriction lets the scheduler skip the hard parts of a general
green-thread runtime, such as per-task stacks and making preemption safe in
arbitrary code, so the mechanics of preemptive switching are easy to follow.

Supported platforms:

| Architecture | glibc | musl |
| --- | --- | --- |
| AArch64 | ✅ | ✅ |
| x86_64 | ✅ | ✅ |

On any other target the binary builds but exits with status 2 and an
"unsupported" message.

## Source layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | Entry point and unsupported-platform fallback |
| `src/round_robin.rs` | Choosing the next unfinished task |
| `src/scheduler/mod.rs` | Builds the worker and runs it |
| `src/scheduler/timer.rs` | Signal handler installation and the POSIX timer |
| `src/scheduler/worker.rs` | The worker, its run loop and the signal handler |
| `src/scheduler/aarch64.rs` | AArch64 guest code and register save/restore |
| `src/scheduler/x86_64.rs` | x86_64 guest code and register save/restore |

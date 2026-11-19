# Design

The scheduler has three parts:

- **The worker** (`scheduler/worker.rs`) owns the tasks, remembers which one
  is running, and holds the alternate signal stack and the timer handle.
- **The timer** (`scheduler/timer.rs`) delivers `SIGALRM` to the worker's
  thread every 2 ms.
- **The architecture module** (`scheduler/aarch64.rs` or
  `scheduler/x86_64.rs`) supplies the guest code, the `Task` register
  layout, and four functions: `enter`, `in_guest`, `save` and `restore`.

`scheduler/mod.rs` uses `#[path]` to pick the architecture module at compile
time, so the worker is written once against that small interface.

## The run loop

```text
while some task is unfinished:
    current = next unfinished task after current
    enter(current)          # returns when *a* guest finishes
    mark current finished   # current may have changed while we were away
delete the timer
```

`enter` saves the Rust caller's callee-saved registers on the main stack,
records the stack pointer in `Worker::main_sp`, loads the task's registers
and jumps to its program counter. When a guest finishes it jumps to
`resume_main`, which restores those registers and returns to the run loop as
if `enter` had returned normally.

Because the timer can switch guests while `enter` is outstanding, the guest
that eventually returns is not necessarily the one the loop entered. The
loop therefore marks `current` as finished, the task the signal handler
last switched to, rather than the index it passed to `enter`.

## The signal handler

On every tick, `Worker::interrupt`:

1. Ignores the signal unless it came from the timer (`SI_TIMER`) and carries
   a pointer to the active worker.
2. Returns without switching unless the interrupted program counter lies
   inside the guest code, between `cruncher` and `guests_end`. A tick that
   lands in the Rust run loop is simply dropped.
3. Saves the interrupted registers into the current task (`arch::save`).
4. Picks the next unfinished task in round-robin order (`next_runnable`).
5. Writes that task's registers into the signal context (`arch::restore`).

When the handler returns, the kernel's `sigreturn` loads the modified context
and the new guest resumes where it was last interrupted, or starts at its
entry point.

## Limitations

- Guests have no stacks of their own; they borrow the frame `enter` left on
  the main stack, so they must not push, call or make system calls.
- Only code between `cruncher` and `guests_end` is preemptible.
- There is one worker on one thread.

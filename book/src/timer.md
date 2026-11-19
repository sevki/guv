# The preemption timer

`Worker::start_timer` sets up preemption in three steps, each in
`scheduler/timer.rs`.

## Installing the handler

`timer::install_handler`:

- registers a 64 KiB alternate signal stack owned by the worker
  (`sigaltstack`), so the handler never runs on the stack the guests borrow;
- installs `Worker::interrupt` for `SIGALRM` with
  `SA_SIGINFO | SA_RESTART | SA_ONSTACK`;
- unblocks `SIGALRM` for the calling thread.

## Creating the timer

`timer::create` calls `timer_create` on `CLOCK_MONOTONIC` with
`SIGEV_THREAD_ID`, aiming the signal at the calling thread rather than at
the whole process. The worker's address goes in `sigev_value`, and the
handler compares it with `ACTIVE_WORKER` before touching any state.

## Arming and tearing down

`timer::arm` makes the timer fire every 2 ms, with the first expiry 2 ms
after arming. If arming fails, the timer is deleted and `ACTIVE_WORKER` is
cleared. When the run loop finishes it calls `timer::delete` and clears
`ACTIVE_WORKER`, so a late tick is ignored.

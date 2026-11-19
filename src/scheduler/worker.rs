//! The worker owns the guests and switches between them from the timer
//! signal handler.

use super::arch::{self, Task};
use super::timer;
use crate::round_robin::next_runnable;
use std::cell::{Cell, UnsafeCell};
use std::io;
use std::sync::atomic::{AtomicPtr, Ordering};

const SIGNAL_STACK_SIZE: usize = std::mem::size_of::<[u128; 4096]>();

pub struct Worker {
    tasks: UnsafeCell<Vec<Task>>,
    current: Cell<usize>,
    main_sp: Cell<usize>,
    signal_stack: UnsafeCell<[u128; 4096]>,
    timer: Cell<libc::timer_t>,
    switches: Cell<usize>,
}

static ACTIVE_WORKER: AtomicPtr<Worker> = AtomicPtr::new(std::ptr::null_mut());

impl Worker {
    pub fn new(tasks: Vec<Task>) -> Box<Self> {
        Box::new(Self {
            current: Cell::new(tasks.len().saturating_sub(1)),
            tasks: UnsafeCell::new(tasks),
            main_sp: Cell::new(0),
            signal_stack: UnsafeCell::new([0; 4096]),
            timer: Cell::new(unsafe { std::mem::zeroed() }),
            switches: Cell::new(0),
        })
    }

    pub unsafe fn start_timer(&self) -> io::Result<()> {
        unsafe {
            timer::install_handler(
                Self::interrupt,
                self.signal_stack.get().cast(),
                SIGNAL_STACK_SIZE,
            )?;
            let handle = timer::create((self as *const Self).cast_mut().cast())?;
            self.timer.set(handle);
            ACTIVE_WORKER.store((self as *const Self).cast_mut(), Ordering::Relaxed);

            if let Err(error) = timer::arm(handle) {
                let _ = timer::delete(handle);
                ACTIVE_WORKER.store(std::ptr::null_mut(), Ordering::Relaxed);
                return Err(error);
            }
        }
        Ok(())
    }

    fn next_task(&self) -> Option<usize> {
        let tasks = unsafe { &*self.tasks.get() };
        next_runnable(self.current.get(), tasks.len(), |index| {
            tasks[index].finished
        })
    }

    pub fn run(&self) -> io::Result<u64> {
        unsafe {
            let mut status = 0;
            while let Some(index) = self.next_task() {
                self.current.set(index);
                let task = (&*self.tasks.get()).as_ptr().add(index);
                status = arch::enter(self.main_sp.as_ptr(), task);
                // The guest that returned is whichever one was running last,
                // which may differ from `index` after preemption.
                (&mut *self.tasks.get())[self.current.get()].finished = true;
            }

            timer::delete(self.timer.get())?;
            ACTIVE_WORKER.store(std::ptr::null_mut(), Ordering::Relaxed);
            Ok(status)
        }
    }

    unsafe extern "C" fn interrupt(
        _: libc::c_int,
        info: *mut libc::siginfo_t,
        state: *mut libc::c_void,
    ) {
        unsafe {
            if info.is_null() || (*info).si_code != libc::SI_TIMER {
                return;
            }
            let signal_worker = (*info).si_value().sival_ptr as *mut Self;
            let active_worker = ACTIVE_WORKER.load(Ordering::Relaxed);
            if active_worker.is_null() || signal_worker != active_worker {
                return;
            }

            let context = &mut (*(state as *mut libc::ucontext_t)).uc_mcontext;
            let worker = &*active_worker;
            if !arch::in_guest(context) {
                return;
            }

            let tasks = &mut *worker.tasks.get();
            arch::save(&mut tasks[worker.current.get()], context);

            let Some(next) = worker.next_task() else {
                return;
            };
            worker.current.set(next);
            worker.switches.set(worker.switches.get() + 1);
            arch::restore(&mut tasks[next], context, worker.main_sp.get());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Worker;
    use crate::scheduler::arch::{Task, cruncher};
    use std::io;

    struct SignalState {
        action: libc::sigaction,
        stack: libc::stack_t,
        mask: libc::sigset_t,
    }

    impl SignalState {
        unsafe fn capture() -> io::Result<Self> {
            unsafe {
                let mut action = std::mem::zeroed();
                let mut stack = std::mem::zeroed();
                let mut mask = std::mem::zeroed();
                if libc::sigaction(libc::SIGALRM, std::ptr::null(), &mut action) != 0
                    || libc::sigaltstack(std::ptr::null(), &mut stack) != 0
                    || libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut mask) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(Self {
                    action,
                    stack,
                    mask,
                })
            }
        }
    }

    impl Drop for SignalState {
        fn drop(&mut self) {
            unsafe {
                let mut alarm = std::mem::zeroed();
                libc::sigemptyset(&mut alarm);
                libc::sigaddset(&mut alarm, libc::SIGALRM);
                libc::sigprocmask(libc::SIG_BLOCK, &alarm, std::ptr::null_mut());
                libc::sigaction(libc::SIGALRM, &self.action, std::ptr::null_mut());
                libc::sigaltstack(&self.stack, std::ptr::null_mut());
                libc::sigprocmask(libc::SIG_SETMASK, &self.mask, std::ptr::null_mut());
            }
        }
    }

    #[test]
    fn timer_preempts_and_completes_each_guest() {
        let worker = Worker::new((0..5).map(|_| Task::new(cruncher)).collect());
        let _signal_state = unsafe { SignalState::capture().unwrap() };
        unsafe { worker.start_timer().unwrap() };

        let status = worker.run().unwrap();
        let switches = worker.switches.get();
        let all_finished = unsafe { &*worker.tasks.get() }
            .iter()
            .all(|task| task.finished);
        assert_eq!(status, 0);
        assert!(switches > 0);
        assert!(all_finished);
    }
}

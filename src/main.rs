#[cfg(any(test, all(target_os = "linux", target_arch = "aarch64")))]
fn next_runnable(
    current: usize,
    task_count: usize,
    mut is_finished: impl FnMut(usize) -> bool,
) -> Option<usize> {
    if task_count == 0 {
        return None;
    }

    (1..=task_count)
        .map(|offset| current.wrapping_add(offset) % task_count)
        .find(|&index| !is_finished(index))
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
mod scheduler {
    use super::next_runnable;
    use std::cell::{Cell, UnsafeCell};
    use std::io;
    use std::sync::atomic::{AtomicPtr, Ordering};

    const TIMER_INTERVAL_NS: libc::c_long = 2_000_000;

    std::arch::global_asm!(
        r#"
        .global cruncher, guests_end, enter_guest, resume_main
        .balign 4
    cruncher:
        ldr  x3, =20000000
        mov  x2, x3
        mul  x0, x3, x3
    guest_loop:
        subs x0, x0, x2
        sub  x3, x3, #1
        cbnz x3, guest_loop
        cset w0, ne
        b    resume_main
    guests_end:

    enter_guest:
        stp  x29, x30, [sp, #-176]!
        stp  x19, x20, [sp, #16]
        stp  x21, x22, [sp, #32]
        stp  x23, x24, [sp, #48]
        stp  x25, x26, [sp, #64]
        stp  x27, x28, [sp, #80]
        mrs  x9, fpcr
        str  x9, [sp, #160]
        mov  x9, sp
        str  x9, [x0]
        add  x9, sp, #96
        st1  {{v8.1d-v11.1d}}, [x9], #32
        st1  {{v12.1d-v15.1d}}, [x9]
        mov  x9, x1
        mov  x10, x2
        ldp  x0, x1, [x10, #8]
        ldp  x2, x3, [x10, #24]
        ldr  x8, [x10, #72]
        ldr  x10, [x10, #264]
        msr  nzcv, x10
        br   x9
    resume_main:
        ldr  x9, [sp, #160]
        msr  fpcr, x9
        add  x9, sp, #96
        ld1  {{v8.1d-v11.1d}}, [x9], #32
        ld1  {{v12.1d-v15.1d}}, [x9]
        ldp  x19, x20, [sp, #16]
        ldp  x21, x22, [sp, #32]
        ldp  x23, x24, [sp, #48]
        ldp  x25, x26, [sp, #64]
        ldp  x27, x28, [sp, #80]
        ldp  x29, x30, [sp], #176
        ret
        "#
    );

    unsafe extern "C" {
        fn cruncher();
        fn guests_end();
        fn enter_guest(main_sp: *mut usize, pc: u64, task: *const Task) -> u64;
    }

    #[repr(C)]
    struct Task {
        pc: u64,
        regs: [u64; 32],
        nzcv: u64,
        finished: bool,
    }

    impl Task {
        fn new(entry: unsafe extern "C" fn()) -> Self {
            Self {
                pc: entry as usize as u64,
                regs: [0; 32],
                nzcv: 0,
                finished: false,
            }
        }
    }

    struct Worker {
        tasks: UnsafeCell<Vec<Task>>,
        current: Cell<usize>,
        main_sp: Cell<usize>,
        signal_stack: UnsafeCell<[u128; 4096]>,
        timer: Cell<libc::timer_t>,
    }

    static ACTIVE_WORKER: AtomicPtr<Worker> = AtomicPtr::new(std::ptr::null_mut());

    impl Worker {
        fn new(tasks: Vec<Task>) -> Box<Self> {
            Box::new(Self {
                current: Cell::new(tasks.len().saturating_sub(1)),
                tasks: UnsafeCell::new(tasks),
                main_sp: Cell::new(0),
                signal_stack: UnsafeCell::new([0; 4096]),
                timer: Cell::new(unsafe { std::mem::zeroed() }),
            })
        }

        unsafe fn start_timer(&self) -> io::Result<()> {
            unsafe {
                let stack = libc::stack_t {
                    ss_sp: self.signal_stack.get().cast(),
                    ss_flags: 0,
                    ss_size: std::mem::size_of::<[u128; 4096]>(),
                };
                if libc::sigaltstack(&stack, std::ptr::null_mut()) != 0 {
                    return Err(io::Error::last_os_error());
                }

                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = Self::interrupt as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(libc::SIGALRM, &action, std::ptr::null_mut()) != 0 {
                    return Err(io::Error::last_os_error());
                }

                let mut unblocked: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut unblocked);
                libc::sigaddset(&mut unblocked, libc::SIGALRM);
                if libc::sigprocmask(libc::SIG_UNBLOCK, &unblocked, std::ptr::null_mut()) != 0 {
                    return Err(io::Error::last_os_error());
                }

                let mut event: libc::sigevent = std::mem::zeroed();
                event.sigev_notify = libc::SIGEV_THREAD_ID;
                event.sigev_signo = libc::SIGALRM;
                event.sigev_value.sival_ptr = (self as *const Self).cast_mut().cast();
                event.sigev_notify_thread_id = libc::gettid();

                let mut timer: libc::timer_t = std::mem::zeroed();
                if libc::timer_create(libc::CLOCK_MONOTONIC, &mut event, &mut timer) != 0 {
                    return Err(io::Error::last_os_error());
                }
                self.timer.set(timer);
                ACTIVE_WORKER.store((self as *const Self).cast_mut(), Ordering::Relaxed);

                let interval = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: TIMER_INTERVAL_NS,
                };
                let spec = libc::itimerspec {
                    it_interval: interval,
                    it_value: interval,
                };
                if libc::timer_settime(timer, 0, &spec, std::ptr::null_mut()) != 0 {
                    let error = io::Error::last_os_error();
                    libc::timer_delete(timer);
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

        fn run(&self) -> io::Result<u64> {
            unsafe {
                let mut status = 0;
                while let Some(index) = self.next_task() {
                    self.current.set(index);
                    let task = (&*self.tasks.get()).as_ptr().add(index);
                    status = enter_guest(self.main_sp.as_ptr(), (*task).pc, task);
                    (&mut *self.tasks.get())[index].finished = true;
                }

                if libc::timer_delete(self.timer.get()) != 0 {
                    return Err(io::Error::last_os_error());
                }
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
                if context.pc < cruncher as *const () as usize as u64
                    || context.pc >= guests_end as *const () as usize as u64
                {
                    return;
                }

                let tasks = &mut *worker.tasks.get();
                let current = worker.current.get();
                tasks[current].pc = context.pc;
                tasks[current].regs[..31].copy_from_slice(&context.regs);
                tasks[current].nzcv = context.pstate;

                let Some(next) = worker.next_task() else {
                    return;
                };
                worker.current.set(next);
                let task = &(&*worker.tasks.get())[next];
                context.pc = task.pc;
                context.regs.copy_from_slice(&task.regs[..31]);
                context.pstate = (context.pstate & !0xf000_0000) | (task.nzcv & 0xf000_0000);
            }
        }
    }

    pub fn run() -> ! {
        let worker = Worker::new((0..5).map(|_| Task::new(cruncher)).collect());
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
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
fn main() {
    scheduler::run()
}

#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
fn main() {
    eprintln!("guv requires Linux on AArch64");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::next_runnable;

    #[test]
    fn round_robin_skips_finished_tasks() {
        let finished = [false, true, true, false];
        assert_eq!(next_runnable(0, finished.len(), |i| finished[i]), Some(3));
        assert_eq!(next_runnable(3, finished.len(), |i| finished[i]), Some(0));
    }

    #[test]
    fn no_task_is_selected_when_all_are_finished_or_empty() {
        assert_eq!(next_runnable(0, 0, |_| false), None);
        assert_eq!(next_runnable(1, 3, |_| true), None);
    }
}

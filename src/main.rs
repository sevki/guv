#[cfg(any(
    test,
    all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "x86_64")
    )
))]
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod scheduler {
    use super::next_runnable;
    use std::cell::{Cell, UnsafeCell};
    use std::io;
    use std::sync::atomic::{AtomicPtr, Ordering};

    const TIMER_INTERVAL_NS: libc::c_long = 2_000_000;
    const STATUS_FLAGS: libc::greg_t = 0x8d5;
    const REGISTER_COUNT: usize = 23;

    std::arch::global_asm!(
        r#"
        .global cruncher, guests_end, enter_guest, resume_main
        .type cruncher,@function
    cruncher:
        movl $10000000, %ecx
        movq %rcx, %rax
        imulq %rcx, %rax
    guest_loop:
        subq %rcx, %rax
        decl %ecx
        jnz guest_loop
        xorl %eax, %eax
        jmp resume_main
    guests_end:
        .size cruncher, guests_end-cruncher

        .type enter_guest,@function
    enter_guest:
        pushq %rbp
        pushq %rbx
        pushq %r12
        pushq %r13
        pushq %r14
        pushq %r15
        movq %rsp, (%rdi)
        movq %rsi, %r11
        pushq 136(%r11)
        popfq
        pushq 128(%r11)
        movq 0(%r11), %r8
        movq 8(%r11), %r9
        movq 16(%r11), %r10
        movq 32(%r11), %r12
        movq 40(%r11), %r13
        movq 48(%r11), %r14
        movq 56(%r11), %r15
        movq 64(%r11), %rdi
        movq 72(%r11), %rsi
        movq 80(%r11), %rbp
        movq 88(%r11), %rbx
        movq 96(%r11), %rdx
        movq 104(%r11), %rax
        movq 112(%r11), %rcx
        movq 24(%r11), %r11
        ret
    resume_main:
        popq %r15
        popq %r14
        popq %r13
        popq %r12
        popq %rbx
        popq %rbp
        ret
        .size enter_guest, .-enter_guest
        "#,
        options(att_syntax)
    );

    unsafe extern "C" {
        fn cruncher();
        fn guests_end();
        fn enter_guest(main_sp: *mut usize, task: *const libc::c_void) -> u64;
    }

    #[repr(C)]
    struct Task {
        regs: [libc::greg_t; REGISTER_COUNT],
        finished: bool,
        started: bool,
    }

    impl Task {
        fn new(entry: unsafe extern "C" fn()) -> Self {
            let mut regs = [0; REGISTER_COUNT];
            regs[libc::REG_RIP as usize] = entry as *const () as usize as libc::greg_t;
            Self {
                regs,
                finished: false,
                started: false,
            }
        }
    }

    struct Worker {
        tasks: UnsafeCell<Vec<Task>>,
        current: Cell<usize>,
        main_sp: Cell<usize>,
        signal_stack: UnsafeCell<[u128; 4096]>,
        timer: Cell<libc::timer_t>,
        switches: Cell<usize>,
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
                switches: Cell::new(0),
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
                    status = enter_guest(self.main_sp.as_ptr(), task.cast());
                    (&mut *self.tasks.get())[self.current.get()].finished = true;
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
                let pc = context.gregs[libc::REG_RIP as usize] as u64;
                if pc < cruncher as *const () as usize as u64
                    || pc >= guests_end as *const () as usize as u64
                {
                    return;
                }

                let tasks = &mut *worker.tasks.get();
                let current = worker.current.get();
                tasks[current].regs.copy_from_slice(&context.gregs);
                tasks[current].started = true;

                let Some(next) = worker.next_task() else {
                    return;
                };
                worker.current.set(next);
                worker.switches.set(worker.switches.get() + 1);
                let task = &mut (&mut *worker.tasks.get())[next];
                if !task.started {
                    let guest_pc = task.regs[libc::REG_RIP as usize];
                    task.regs.copy_from_slice(&context.gregs);
                    task.regs[libc::REG_RIP as usize] = guest_pc;
                    task.regs[libc::REG_RSP as usize] = worker.main_sp.get() as libc::greg_t;
                    task.started = true;
                }

                let current_flags = context.gregs[libc::REG_EFL as usize];
                let task_flags = task.regs[libc::REG_EFL as usize];
                context.gregs.copy_from_slice(&task.regs);
                context.gregs[libc::REG_EFL as usize] =
                    (current_flags & !STATUS_FLAGS) | (task_flags & STATUS_FLAGS);
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

    #[cfg(test)]
    mod tests {
        use super::{Task, Worker, cruncher};
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
}

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

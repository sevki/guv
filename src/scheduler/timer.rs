//! Signal-handler installation and the thread-directed preemption timer.

use std::io;

const TIMER_INTERVAL_NS: libc::c_long = 2_000_000;

pub type Handler = unsafe extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void);

/// Installs `handler` for `SIGALRM` on the alternate stack and unblocks it.
pub unsafe fn install_handler(
    handler: Handler,
    stack: *mut u8,
    stack_size: usize,
) -> io::Result<()> {
    unsafe {
        let stack = libc::stack_t {
            ss_sp: stack.cast(),
            ss_flags: 0,
            ss_size: stack_size,
        };
        if libc::sigaltstack(&stack, std::ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as *const () as usize;
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
    }
    Ok(())
}

/// Creates a disarmed timer that delivers `SIGALRM` carrying `value` to the
/// calling thread.
pub unsafe fn create(value: *mut libc::c_void) -> io::Result<libc::timer_t> {
    unsafe {
        let mut event: libc::sigevent = std::mem::zeroed();
        event.sigev_notify = libc::SIGEV_THREAD_ID;
        event.sigev_signo = libc::SIGALRM;
        event.sigev_value.sival_ptr = value;
        event.sigev_notify_thread_id = libc::gettid();

        let mut timer: libc::timer_t = std::mem::zeroed();
        if libc::timer_create(libc::CLOCK_MONOTONIC, &mut event, &mut timer) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(timer)
    }
}

/// Arms `timer` to fire periodically at the preemption interval.
pub unsafe fn arm(timer: libc::timer_t) -> io::Result<()> {
    let interval = libc::timespec {
        tv_sec: 0,
        tv_nsec: TIMER_INTERVAL_NS,
    };
    let spec = libc::itimerspec {
        it_interval: interval,
        it_value: interval,
    };
    if unsafe { libc::timer_settime(timer, 0, &spec, std::ptr::null_mut()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub unsafe fn delete(timer: libc::timer_t) -> io::Result<()> {
    if unsafe { libc::timer_delete(timer) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

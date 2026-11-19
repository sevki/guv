//! x86_64 guest code and context switching.

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
    pub fn cruncher();
    fn guests_end();
    fn enter_guest(main_sp: *mut usize, task: *const libc::c_void) -> u64;
}

const STATUS_FLAGS: libc::greg_t = 0x8d5;
const REGISTER_COUNT: usize = 23;
const RIP: usize = libc::REG_RIP as usize;
const RSP: usize = libc::REG_RSP as usize;
const EFL: usize = libc::REG_EFL as usize;

/// Saved guest state in `gregs` order. `enter_guest` reads this layout
/// directly.
#[repr(C)]
pub struct Task {
    regs: [libc::greg_t; REGISTER_COUNT],
    pub finished: bool,
    started: bool,
}

impl Task {
    pub fn new(entry: unsafe extern "C" fn()) -> Self {
        let mut regs = [0; REGISTER_COUNT];
        regs[RIP] = entry as *const () as usize as libc::greg_t;
        Self {
            regs,
            finished: false,
            started: false,
        }
    }
}

/// Starts or resumes `task` and returns when a guest finishes.
pub unsafe fn enter(main_sp: *mut usize, task: *const Task) -> u64 {
    unsafe { enter_guest(main_sp, task.cast()) }
}

pub fn in_guest(context: &libc::mcontext_t) -> bool {
    let start = cruncher as *const () as usize as u64;
    let end = guests_end as *const () as usize as u64;
    (start..end).contains(&(context.gregs[RIP] as u64))
}

pub fn save(task: &mut Task, context: &libc::mcontext_t) {
    task.regs.copy_from_slice(&context.gregs);
    task.started = true;
}

pub fn restore(task: &mut Task, context: &mut libc::mcontext_t, main_sp: usize) {
    if !task.started {
        let guest_pc = task.regs[RIP];
        task.regs.copy_from_slice(&context.gregs);
        task.regs[RIP] = guest_pc;
        task.regs[RSP] = main_sp as libc::greg_t;
        task.started = true;
    }

    let current_flags = context.gregs[EFL];
    let task_flags = task.regs[EFL];
    context.gregs.copy_from_slice(&task.regs);
    context.gregs[EFL] = (current_flags & !STATUS_FLAGS) | (task_flags & STATUS_FLAGS);
}

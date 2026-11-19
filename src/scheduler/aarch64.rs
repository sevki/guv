//! AArch64 guest code and context switching.

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
    pub fn cruncher();
    fn guests_end();
    fn enter_guest(main_sp: *mut usize, pc: u64, task: *const Task) -> u64;
}

const NZCV_MASK: u64 = 0xf000_0000;

/// Saved guest state. `enter_guest` reads this layout directly.
#[repr(C)]
pub struct Task {
    pc: u64,
    regs: [u64; 32],
    nzcv: u64,
    pub finished: bool,
}

impl Task {
    pub fn new(entry: unsafe extern "C" fn()) -> Self {
        Self {
            pc: entry as usize as u64,
            regs: [0; 32],
            nzcv: 0,
            finished: false,
        }
    }
}

/// Starts or resumes `task` and returns when a guest finishes.
pub unsafe fn enter(main_sp: *mut usize, task: *const Task) -> u64 {
    unsafe { enter_guest(main_sp, (*task).pc, task) }
}

pub fn in_guest(context: &libc::mcontext_t) -> bool {
    let start = cruncher as *const () as usize as u64;
    let end = guests_end as *const () as usize as u64;
    (start..end).contains(&context.pc)
}

pub fn save(task: &mut Task, context: &libc::mcontext_t) {
    task.pc = context.pc;
    task.regs[..31].copy_from_slice(&context.regs);
    task.nzcv = context.pstate;
}

pub fn restore(task: &mut Task, context: &mut libc::mcontext_t, _main_sp: usize) {
    context.pc = task.pc;
    context.regs.copy_from_slice(&task.regs[..31]);
    context.pstate = (context.pstate & !NZCV_MASK) | (task.nzcv & NZCV_MASK);
}

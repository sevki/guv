# AArch64

## Guest

`cruncher` squares 20,000,000 into `x0`, then subtracts the original value
once per iteration while counting `x3` down to zero. The result should be
zero; `cset w0, ne` turns any other result into a non-zero exit status.

## Task layout

```rust,ignore
#[repr(C)]
pub struct Task {
    pc: u64,          // offset 0
    regs: [u64; 32],  // offset 8: x0..x30 (x31 unused)
    nzcv: u64,        // offset 264
    pub finished: bool,
}
```

## Entering

`enter_guest(main_sp, pc, task)` builds a 176-byte frame holding `x19`–`x30`,
the low halves of `v8`–`v15` and `fpcr`. It stores `sp` through `main_sp`,
loads the guest's `x0`–`x3`, `x8` and NZCV from the task, and branches to
`pc`. `resume_main` unwinds that frame and returns.

## Switching

- `save` copies `pc`, `x0`–`x30` and `pstate` from the signal context.
- `restore` writes `pc` and `x0`–`x30` back, and replaces only the NZCV bits
  (`0xf000_0000`) of `pstate`.

The guest never moves `sp`, so the stack pointer in the context is already
correct for every guest and is not swapped.

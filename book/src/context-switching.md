# Context switching

Each architecture module exposes the same interface to the worker:

| Item | Role |
| --- | --- |
| `Task` | `#[repr(C)]` saved registers plus a `finished` flag; `enter_guest` reads its fields by offset |
| `cruncher` | The guest entry point |
| `enter(main_sp, task)` | Starts or resumes a task from the run loop |
| `in_guest(ctx)` | Whether the interrupted PC lies in guest code |
| `save(task, ctx)` | Copies the interrupted registers into the task |
| `restore(task, ctx, main_sp)` | Copies the task's registers into the context |

Both modules preserve the kernel's control bits in the signal context and
only swap the arithmetic condition flags, so a guest cannot corrupt
processor state it does not own.

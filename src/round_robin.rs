//! Architecture-independent task selection.

/// Returns the next unfinished task after `current`, wrapping around.
#[cfg_attr(
    not(all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "x86_64")
    )),
    allow(dead_code)
)]
pub fn next_runnable(
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

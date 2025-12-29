//
// BlightOS Kernel
//
// Round-Robin Task Scheduler
//

use crate::arch;

pub fn select_next(tasks: &mut [arch::TaskContext], cur: usize, idle: usize) -> usize {
    // pick the next runnable task
    for &mut t in &mut tasks[cur+1..] {
        if t.runnable() && t.tid() != idle {
            return t.tid();
        }
    }
    // hit the end of the pool, start from the beginning
    for &mut t in &mut tasks[0..=cur] {
        if t.runnable() && t.tid() != idle {
            return t.tid();
        }
    }
    // No runnable task found. Return the idle task
    idle
}
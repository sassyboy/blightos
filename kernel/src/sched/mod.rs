//
// BlightOS Kernel
//
// Task Scheduler
//   Provides an interface to the kernel code for switching between tasks and
//   address spaces based on the selected scheduler, and the target architecture
//
// Todo - preemption lock!
use crate::arch;
mod sched_rr;

const TASK_POOL_CAP: usize = 8;
const TASK_STACK_SIZE: usize = 1024; // In terms of usize
static mut TASK_STACKS : [usize; TASK_STACK_SIZE * TASK_POOL_CAP] = 
                            [0; TASK_STACK_SIZE * TASK_POOL_CAP];
static mut TASK_POOL: [arch::TaskContext; TASK_POOL_CAP] = 
                            [arch::TaskContext::new(); TASK_POOL_CAP];
static mut CURRENT: usize = 0;

pub fn new_task(id: usize, func: fn()) {
    if id < TASK_POOL_CAP {
        unsafe {
            TASK_POOL[id].init(id, func, 
                &mut TASK_STACKS[id*TASK_STACK_SIZE..(id+1)*TASK_STACK_SIZE]);
        }
    }
}

// Terminates the current task
pub fn terminate_task() {
    preempt();
}

pub fn start_scheduling(id: usize) {
    unsafe {
        if id < TASK_POOL_CAP && TASK_POOL[id].runnable() {
            CURRENT = id;
            arch::cpu_start_first_task(&mut TASK_POOL[CURRENT]);
        }
    }
}

fn yield_to(task: usize){
    unsafe {
        let old = CURRENT;
        CURRENT = task;
        arch::cpu_switch_context(&mut TASK_POOL[old], &mut TASK_POOL[CURRENT]);
    }
}

pub fn preempt() {
    unsafe {
        let next_task = 
            sched_rr::select_next(&mut TASK_POOL[..], CURRENT, 0);
        if next_task != CURRENT {
            yield_to(next_task);
        }
    }
}

// Structure to implement-free code sections, Rust style!
//
// Usage Example:
// if let Ok(_lock) = sched::Preemption::lock() {
//   ... Code that runs with preemption disabled on the current CPU ...
// }
// ... Preemption back on after _lock goes out of scope!
pub struct Preemption;
impl Preemption {
    pub fn lock() -> Result<Self, ()> {
        arch::cpu_disable_ints();
        Ok(Self {})
    }
}
impl Drop for Preemption {
    fn drop(&mut self) {
        arch::cpu_enable_ints();
    }
}


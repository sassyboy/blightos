use core::panic;

//
// BlightOS Kernel
//
// Task Scheduler
//   Provides an interface to the kernel code for switching between tasks and
//   address spaces based on the selected scheduler, and the target architecture
//
use crate::arch::{self, SysTimerDuration};
use crate::arch::THIS_CPU_ID;
use crate::pmm::{PHY_FRAME_SIZE, palloc_continuous, pfree};
use crate::sched::sched_rr::RoundRobinScheduler;
use crate::util::*;
use core::slice::*;

mod sched_rr;

const PERCPU_TASK_POOL_CAP: usize = 8;
// Each CPU has its own task pool
// Migration is not supported.
// Todo: Need a dispatcher/idle task for each CPU
//       CPUx should be able to launch a task in CPUy (think of a shell program)
percpu_global! {
    CURRENT_TASK:   usize = 0; // currently running task id (local id)
    IDLE_TASK:      usize = 0; // idle task id of this CPU
    TASK_POOL:      [Task; PERCPU_TASK_POOL_CAP] = 
                    [Task::new() ; PERCPU_TASK_POOL_CAP];
    pub SCHEDULER:  Scheduler = Scheduler::new();
}


// Generic Task Structure
// Todo: - Move unnecessary state info from arch-dependent context to here
//       - Process (Task group) information
//       - Blocked/Event Queues
#[derive(Clone, Copy)]
pub struct Task {
    tid:            usize, // Index in the percpu pool
    gtid:           usize, // Index in the system (to locate stack, etc.)
    valid:          bool,
    // CPU-dependent Runtime Context
    pub ctx:        arch::TaskContext,
    pub cpu:        u64, // Todo use as a mask, but for now run on 1 cpu only
    //
    stack_base:     usize,
    stack_pages:    usize,
    // Priority fields for RMS, FQS (b/p), etc
    _sched_p1:      u64,
    _sched_p2:      u64,
    _sched_p3:      u64
}
impl Task {
    pub const fn new() -> Self {
        Self {
            tid:        0,
            gtid:       0,
            valid:      false,
            ctx:        arch::TaskContext::new(),
            cpu:        0,
            stack_base: 0,
            stack_pages:0,
            _sched_p1:  0,
            _sched_p2:  0,
            _sched_p3:  0
        }
    }
    pub fn runnable(&self) -> bool {
        self.ctx.runnable() && self.valid
    }
    pub fn tid(&self) -> usize {
        self.tid
    }
    pub fn gtid(&self) -> usize {
        self.gtid
    }
}

//
// Generic Task Scheduler
//
enum SchedulerOps{
    FirstComeFirstServe(FcfsScheduler),
    RoundRobin(RoundRobinScheduler),
    // Other policies go here
    // Rate-Monotonic
    // EDF
}

pub struct Scheduler {
    ops:    SchedulerOps,
}
impl Scheduler {
    pub const fn new() -> Self {
        Self {
            ops: SchedulerOps::FirstComeFirstServe(FcfsScheduler::new())
        }
    }

    // Policy selection routines
    pub fn config_first_come_first_served(&mut self) {
        self.ops = SchedulerOps::FirstComeFirstServe(FcfsScheduler::new());
    }
    pub fn config_round_robin(&mut self, quantum: SysTimerDuration) {
        self.ops = SchedulerOps::RoundRobin(RoundRobinScheduler::new());
        if let SchedulerOps::RoundRobin(rr) = &mut self.ops {
            rr.init(quantum);
        }
    }

    //
    pub fn create_task(&self, id: usize, func: fn(), stack_pgs: usize) -> bool {
        if id >= PERCPU_TASK_POOL_CAP {
            return false; // Out of memory
        }
    
        let cpuid = THIS_CPU_ID.borrow();
        let gtid  = cpuid * PERCPU_TASK_POOL_CAP + id; // global task id
        let tpool = TASK_POOL.borrow_mut();
        if tpool[id].valid == true {
            return false; // Task already exists
        }
        if let Some(stack) = self.stack_alloc(stack_pgs) {

            tpool[id].tid   = id;
            tpool[id].gtid  = gtid;
            tpool[id].cpu   = *cpuid as u64;
            tpool[id].stack_base = stack.as_mut_ptr() as usize;
            tpool[id].stack_pages = stack.len() * 
                                    size_of::<usize>() / PHY_FRAME_SIZE;
            tpool[id].ctx.init(gtid, func, stack);
            tpool[id].valid = true;
            return true;
        }
        false
    }

    // Terminates the current task
    pub fn terminate_task(&self) {
        // Release the task resources
        let cur_tid = *(CURRENT_TASK.borrow());
        let tpool   = TASK_POOL.borrow_mut();
        self.stack_free(&mut tpool[cur_tid]);
        tpool[cur_tid].valid = false;
        // Move onto another task
        self.preempt();
    }

    // Start the scheduler on the current CPU starting with local task: id
    // Swithing to the first tast will also enable interrupts on the CPU
    pub fn start_scheduling(&self, starting_id: usize, idle_task_id: usize) {
        if starting_id >= PERCPU_TASK_POOL_CAP {
            panic!("Cannot start the scheduler with stid {}", starting_id);
        }
        IDLE_TASK.write(idle_task_id);
        let tpool = TASK_POOL.borrow_mut();
        if tpool[idle_task_id].runnable() == false {
            panic!("Cannot start the scheduler without a proper idle task");
        }   
        if tpool[starting_id].ctx.runnable() {
            CURRENT_TASK.write(starting_id);
            arch::cpu_start_first_task(&mut tpool[starting_id].ctx);
        } else {
            panic!("Cannot start the scheduler without a proper starting task");
        }
    }

    fn yield_to(&self, id: usize){
        if id >= PERCPU_TASK_POOL_CAP {
            return;
        }

        let tpool = TASK_POOL.borrow_mut();
        if tpool[id].valid == false || tpool[id].ctx.runnable() == false {
            return;
        }
        let old = *(CURRENT_TASK.borrow());
        CURRENT_TASK.write(id);
    
        arch::cpu_switch_context(&(tpool[old].ctx), &(tpool[id].ctx));
    }

    pub fn preempt(&self) {
        let tpool   = TASK_POOL.borrow_mut();
        let cur_tid = *(CURRENT_TASK.borrow());
        let idl_tid = *(IDLE_TASK.borrow());
        let next_task;
        match &self.ops {
            SchedulerOps::FirstComeFirstServe(fcfs) => {
                next_task = fcfs.next_task(tpool, cur_tid, idl_tid);
            }
            SchedulerOps::RoundRobin(rr) => {
                next_task = rr.next_task(tpool, cur_tid, idl_tid);
            }
        };

        if next_task != cur_tid {
            self.yield_to(next_task);
        }
    }

    fn stack_alloc(&self, stack_pages: usize) -> Option<&mut [usize]> {
        if let Some(base_addr) = palloc_continuous(stack_pages) {
            let stack: &mut [usize];
            unsafe {
                stack = from_raw_parts_mut(base_addr as *mut usize, 
                            stack_pages * PHY_FRAME_SIZE / size_of::<usize>());
            }
            return Some(stack);
        }
        None
    }

    fn stack_free(&self, task: &mut Task) {
        for i in 0..task.stack_pages {
            pfree(task.stack_base + i * PHY_FRAME_SIZE);
        }
    }
} 

//
// Default Scheduler : FCFS - Runs tasks to completion in order
//

pub struct FcfsScheduler;
impl FcfsScheduler {
    pub const fn new() -> Self {
        Self {}
    }
    pub fn next_task(&self, tasks: &mut [Task], _cur: usize, idle: usize) 
                    -> usize
    {
        // pick the first runnable task
        for &mut t in tasks {
            if t.runnable() && t.tid() != idle {
                return t.tid();
            }
        }
        idle
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


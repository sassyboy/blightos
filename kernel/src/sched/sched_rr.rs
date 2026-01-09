//
// BlightOS Kernel
//
// Round-Robin Task Scheduler
//

use crate::{arch::{SysTimerDuration, THIS_CPU_SYSTIMER}, sched::*};


pub struct RoundRobinScheduler{
    quantum_us : SysTimerDuration,
}

impl RoundRobinScheduler {
    pub const fn new() -> Self{
        Self {
            quantum_us: SysTimerDuration::Ticks(0)
        }
    }

    pub fn init(&mut self, q_us: SysTimerDuration){
        self.quantum_us = q_us;
        let systimer = THIS_CPU_SYSTIMER.borrow_mut();
        systimer.set_mode(arch::SysTimerMode::OneShot);
        systimer.arm(self.quantum_us);
    }
    
    pub fn next_task(&self, tasks: &mut [Task], cur: usize, idle: usize)
                    -> usize
    {
        // pick the next runnable task
        for &mut t in &mut tasks[cur+1..] {
            if t.runnable() && t.tid() != idle {
                THIS_CPU_SYSTIMER.borrow().arm(self.quantum_us);
                return t.tid();
            }
        }
        // hit the end of the pool, start from the beginning
        for &mut t in &mut tasks[0..=cur] {
            if t.runnable() && t.tid() != idle {
                THIS_CPU_SYSTIMER.borrow().arm(self.quantum_us);
                return t.tid();
            }
        }
        // No runnable task found. Return the idle task
        idle
    }
    
}    

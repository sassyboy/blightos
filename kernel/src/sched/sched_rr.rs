//
// BlightOS Kernel
//
// Round-Robin Task Scheduler
//
use alloc::collections::LinkedList;
use crate::arch::SystemTimer;
use crate::sched::*;


//
// Default Scheduler : FCFS - Runs tasks to completion in order
//
#[derive(Debug)]
pub struct FcfsScheduler {
    run_queue:   LinkedList<usize>
}
impl FcfsScheduler {
    pub const fn new() -> Self {
        Self {
            run_queue: LinkedList::new()
        }
    }

    pub fn add_task(&mut self, tid: usize) {
        self.run_queue.push_back(tid);
    }

    pub fn rem_task(&mut self, tid: usize) {
        self.run_queue.retain(|x| *x != tid);
    }

    pub fn next_task(&mut self) -> Option<usize>
    {
        match self.run_queue.pop_front() {
            Some(tid)      => {
                self.run_queue.push_front(tid);
                Some(tid)
            }
            None            => None
        }       
    }
}

#[derive(Debug)]
pub struct RoundRobinScheduler{
    quantum :   Duration,
    run_queue:  LinkedList<usize>
}

impl RoundRobinScheduler {
    pub const fn new() -> Self{
        Self {
            quantum:    Duration::from_micros(0),
            run_queue:  LinkedList::new()
        }
    }

    pub fn init(&mut self, quantum: Duration){
        self.quantum = quantum;
        SystemTimer::set_mode(arch::SysTimerMode::OneShot);
        SystemTimer::arm(self.quantum);
    }
    
    pub fn add_task(&mut self, tid: usize) {
        self.run_queue.push_back(tid);
    }

    pub fn rem_task(&mut self, tid: usize) {
        self.run_queue.retain(|x| *x != tid);
        // klog!("..{:?}..", self.run_queue);
    }

    pub fn next_task(&mut self) -> Option<usize>
    {
        match self.run_queue.pop_front() {
            Some(tid)       => {
                self.run_queue.push_back(tid);
                SystemTimer::arm(self.quantum);
                Some(tid)
            }
            None            => None
        }           
    }
    
}    

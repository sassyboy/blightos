//
// BlightOS Kernel
//
// Task Scheduler
//   Provides an interface to the kernel code for switching between tasks and
//   address spaces based on the selected scheduler, and the target architecture
//
use crate::arch::{self, SysTimerDuration};
use crate::arch::THIS_CPU_ID;
use crate::mem::phys::*;
use crate::mem::virt::AddressSpace;
use crate::sched::sched_rr::FcfsScheduler;
use crate::sched::sched_rr::RoundRobinScheduler;
use crate::util::*;
use core::fmt;
use core::fmt::Display;
use core::slice::*;
use core::time::Duration;
use alloc::collections::BTreeMap;
use alloc::collections::linked_list::LinkedList;
use core::panic;
use core::sync::atomic::{AtomicUsize, Ordering};


// Serial Port Debugging
#[cfg(feature="debug_sched")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[SCHED] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}
#[cfg(not(feature="debug_sched"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}

mod sched_rr;

// Each CPU has its own task pool
// Migration is not supported.
// Todo: Need a dispatcher/idle task for each CPU
//       CPUx should be able to launch a task in CPUy (think of a shell program)
percpu_global! {
    CURR_TID:       usize = 0; // Currently running task
    TASK_POOL:      BTreeMap<usize, Task> = BTreeMap::new();
    pub SCHEDULER:  Scheduler = Scheduler::new();
}

static DEFAULT_STACK_SIZE:  AtomicUsize = AtomicUsize::new(PHY_FRAME_SIZE * 2);
static NEXT_TASK_ID:        AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, PartialEq, PartialOrd)]
#[repr(usize)]
pub enum TaskState {
    New,            // Struct allocated but not initalized
    Ready,          // Runnable - In a run queue but not running atm
    Running,        // Currently running task
    Blocked,        // Waiting for an event (join, wait channel signal, etc)
    Terminating,    // Exited/Died but waiting for resource (stack,etc) dealloc.
    Dropped,        // Deallocated but the object is still around
}
impl Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskState::New          => write!(f, "New"),
            TaskState::Ready        => write!(f, "Ready"),
            TaskState::Running      => write!(f, "Running"),
            TaskState::Blocked      => write!(f, "Blocked"),
            TaskState::Terminating  => write!(f, "Terminating"),
            TaskState::Dropped      => write!(f, "Dropped"),
        }
        
    }
}
// Generic Task Structure
// Todo: - Move unnecessary state info from arch-dependent context to here
//       - Process (Task group) information
//       - Blocked/Event Queues

#[derive(Clone, Debug)]
pub struct Task {
    tid:            usize, // Task ID
    pid:            usize, // Process Address Space ID (0: Kernel-space/no proc)
    pub state:      TaskState,
    pub cpu:        u64, // Todo use as a mask, but for now run on 1 cpu only
    //
    stack_base:     usize,
    stack_pages:    usize,
    // Priority fields for RMS, FQS (b/p), etc
    _sched_p1:      u64,
    _sched_p2:      u64,
    _sched_p3:      u64,
    // List of tasks waiting/blocked on this one to finish
    joiners:        LinkedList<usize>,
    // CPU-dependent Runtime Context
    pub ctx:        arch::TaskContext,

}
impl Task {
    
    pub const fn new() -> Self {
        Self {
            tid:        0,
            pid:        0,
            state:      TaskState::New,
            ctx:        arch::TaskContext::new(),
            cpu:        0,
            stack_base: 0,
            stack_pages:0,
            _sched_p1:  0,
            _sched_p2:  0,
            _sched_p3:  0,
            joiners:    LinkedList::new()
        }
    }
    pub fn runnable(&self) -> bool {
        self.state == TaskState::Ready || self.state == TaskState::Running
    }
    pub fn tid(&self) -> usize {
        self.tid
    }
    pub fn pid(&self) -> usize {
        self.pid
    }

    pub fn current() -> usize {
        *(CURR_TID.borrow())
    }

    pub fn set_default_stack_size(size_bytes: usize) {
        DEFAULT_STACK_SIZE.store(size_bytes, Ordering::Relaxed);
    }

    //
    // Creates a task with the default stack size and returns the gtid of the
    // new task
    //
    pub fn spawn(func: fn()) -> usize {
        let sched = SCHEDULER.borrow_mut();
        let stack_pgs = round_up!(DEFAULT_STACK_SIZE.load(Ordering::Relaxed),
                        PHY_FRAME_SIZE) / PHY_FRAME_SIZE;
        sched.create_task(func, stack_pgs)
    }

    pub fn preempt(){
        arch::cpu_trigger_systimer_irq();
    }

    pub fn join(tid: usize) {
        if let Ok(_lock) = Preemption::lock() {
            let curr_tid = *(CURR_TID.borrow());
            // klog!("TASK {} BLOCKING ON {}'s Termination\n", curr_tid, tid);
            let tpool = TASK_POOL.borrow_mut();
            match tpool.get_mut(&tid) {
                Some(target_task)  => {
                    target_task.joiners.push_back(curr_tid);
                    SCHEDULER.borrow_mut().block_task(curr_tid);
                },
                None        => {} // The task in question doesn't exist
            }
        }
    }

    pub fn sleep(_d: Duration){
        // TODO implement a non-busywaiting sleep
        arch::cpu_busywait_us(_d.as_micros() as u64);
    }

    // Unblocks a task waiting on a wait channel or sleeping
    pub fn wake(tid: usize) {
        if let Ok(_lock) = Preemption::lock() {
            SCHEDULER.borrow_mut().unblock_task(tid);
        }
    }

    pub fn exit() {
        SCHEDULER.borrow_mut().terminate_task();
    }

    pub fn migrate_to_process(pid: usize) {
        if let Ok(_lock) = Preemption::lock() {
            let tid = *(CURR_TID.borrow());
            let tpool = TASK_POOL.borrow_mut();
            match tpool.get_mut(&tid) {
                Some(task)      => {
                    if task.pid == 0 {
                        task.pid = pid
                    } else {
                        panic!("Can't migrate to a new process address space.");
                    }
                },
                None            => {}
            }
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        // Signal the tasks waiting on this one
        for wtid in self.joiners.iter() {
            SCHEDULER.borrow_mut().unblock_task(*wtid);
        }
        // Notify the process about this task being dropped
        AddressSpace::task_dropped(self.pid, self.tid);
        // Free the kernel stack
        pfree_continuous(self.stack_base, self.stack_pages);
        self.state = TaskState::Dropped;
        dbg!("Dropped task {} - Free frames: {}\n",
                self.tid, pmm_num_free_frames());
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

    //
    // Class methods
    //
    // Policy selection routines
    pub fn config_first_come_first_served() {
        let sched = SCHEDULER.borrow_mut();
        sched.ops = SchedulerOps::FirstComeFirstServe(FcfsScheduler::new());
    }

    pub fn config_round_robin(quantum: SysTimerDuration) {
        let sched = SCHEDULER.borrow_mut();
        sched.ops = SchedulerOps::RoundRobin(RoundRobinScheduler::new());
        if let SchedulerOps::RoundRobin(rr) = &mut sched.ops {
            rr.init(quantum);
        }
    }

    // Start the scheduler on the current CPU starting with local task: id
    // Swithing to the first tast will also enable interrupts on the CPU
    pub fn start_scheduling(&mut self, starting_tid: usize) {
        let start_task = TASK_POOL.borrow_mut().get_mut(&starting_tid)
            .expect("Cannot start the scheduler without a starting task");
        if start_task.runnable() == false {
            panic!("The starting task is not runnable");
        }
        // Set the current task to the starting task
        CURR_TID.write(starting_tid);

        // Add the percpu worker scheduler task
        let _pcpuw = self.create_task(Self::percpu_sched_worker, 1);

        // Switch to the starting task!
        start_task.state = TaskState::Running;
        dbg!("Starting on CPU {} w\\ TID: {}(s:{}), PCPUWorkerTID: {}(s:{})\n",
            *(THIS_CPU_ID.borrow()),
            starting_tid, start_task.state,
            _pcpuw, TASK_POOL.borrow_mut().get_mut(&_pcpuw).unwrap().state);
        arch::cpu_switch_context_nosave(&start_task.ctx);
    }

    // Returns the tid of the task created
    pub fn create_task(&mut self, func: fn(), stack_pgs: usize) -> usize {
        if let Ok(_lock) = Preemption::lock() {
            // Changing the per-cpu pool -> Disable Preemption
            if let Some(stack) = self.stack_alloc(stack_pgs) {
                let cpuid = *(THIS_CPU_ID.borrow());
                let tid   = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
                let tpool = TASK_POOL.borrow_mut();
                let new_task;
                tpool.insert(tid, Task::new());
                new_task = tpool.get_mut(&tid).expect("Task pool failure");
                new_task.tid            = tid;
                new_task.cpu            = cpuid as u64;
                new_task.stack_base     = stack.as_mut_ptr() as usize;
                new_task.stack_pages    = stack.len() * 
                                            size_of::<usize>() / PHY_FRAME_SIZE;
                new_task.ctx.init(tid, func, stack);
                dbg!("Created Task TID:{}, stack: {:p}, state: {}\n",
                    new_task.tid, stack, new_task.state);
                match &mut self.ops {
                    SchedulerOps::FirstComeFirstServe(fcfs) => {
                        fcfs.add_task(tid);
                        new_task.state = TaskState::Ready;
                        dbg!("Run Queue after adding {}: {:?}\n", tid, fcfs);
                    }
                    SchedulerOps::RoundRobin(rr) => {
                        rr.add_task(tid);
                        new_task.state = TaskState::Ready;
                        dbg!("Run Queue after adding {}: {:?}\n", tid, rr);
                    }
                };
                return tid;
            }
        }
        0
    }

    // Terminates the current task
    pub fn terminate_task(&mut self) {
        // Release the task resources
        if let Ok(_lock) = Preemption::lock() {
            let term_tid = *(CURR_TID.borrow());
            dbg!("Terminating TID {}\n", term_tid);
            // Changing the per-cpu pool -> Disable Preemption
            match &mut self.ops {
                SchedulerOps::FirstComeFirstServe(fcfs) => {
                    fcfs.rem_task(term_tid);
                    dbg!("Run Queue after Terminating {}: {:?}\n", term_tid, fcfs);
                }
                SchedulerOps::RoundRobin(rr) => {
                    rr.rem_task(term_tid);
                    dbg!("Run Queue after Terminating {}: {:?}\n", term_tid, rr);
                }
            };
            TASK_POOL.borrow_mut().get_mut(&term_tid)
                .unwrap().state = TaskState::Terminating;
            arch::cpu_trigger_systimer_irq();
        }
    }

    // Should be only called in an irq context
    pub fn preempt_irq(&mut self) {
        let cur_task = *(CURR_TID.borrow()); // Put the outgoing TID on the stack
        let next_task;
        let next_option;
        match &mut self.ops {
            SchedulerOps::FirstComeFirstServe(fcfs) => {
                next_option = fcfs.next_task();
            }
            SchedulerOps::RoundRobin(rr) => {
                next_option = rr.next_task();
            }
        };
        match next_option {
            Some(tid)   => {
                next_task = TASK_POOL.borrow_mut().get_mut(&tid)
                            .expect("Task not found");
            }
            None        => {
                panic!("The run queue should never get empty!");
            }
        }

        if let Some(old) = TASK_POOL.borrow_mut().get_mut(&cur_task) {
            if next_task.runnable() == false {
                dbg!("BUG - Invalid next task: TID:{}, state: {}, stack:{:X}\n",
                next_task.tid, next_task.state, next_task.stack_base);
            }
            CURR_TID.write(next_task.tid);
            if old.tid != next_task.tid {
                if old.state == TaskState::Running {
                    old.state = TaskState::Ready;
                }
                next_task.state = TaskState::Running;
                dbg!("Preempt {}(s:{}) -> {}(s:{})\n",
                    old.tid, old.state, next_task.tid, next_task.state);
                arch::cpu_switch_context(&(old.ctx), &(next_task.ctx));
                dbg!("   RETURN FROM PREEMPT: TID {}\n", cur_task);
            }  
        } else {
            panic!("Returned to the deleted context of TID {}\n", cur_task);
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

    pub fn block_task(&mut self, tid: usize) {
        if let Ok(_lock) = Preemption::lock() {
            // Changing the per-cpu pool -> Disable Preemption
            dbg!("-BLOCKING TID {}\n", tid);
            TASK_POOL.borrow_mut().get_mut(&tid).expect("Task not found")
                    .state = TaskState::Blocked;
            match &mut self.ops {
                SchedulerOps::FirstComeFirstServe(fcfs) => {
                    fcfs.rem_task(tid);
                    dbg!("Run Queue after Blocking {}: {:?}\n", tid, fcfs);
                }
                SchedulerOps::RoundRobin(rr) => {
                    rr.rem_task(tid);
                    dbg!("Run Queue after Blocking {}: {:?}\n", tid, rr);
                }
            };
            arch::cpu_trigger_systimer_irq();
        }
    }

    pub fn unblock_task(&mut self, tid: usize) {
        if let Ok(_lock) = Preemption::lock() {
            dbg!("+UNBLOCKING TID {}\n", tid);
            TASK_POOL.borrow_mut().get_mut(&tid).expect("Task not found")
                    .state = TaskState::Ready;
            match &mut self.ops {
                SchedulerOps::FirstComeFirstServe(fcfs) => {
                    fcfs.add_task(tid);
                    dbg!("Run Queue after adding {}: {:?}\n", tid, fcfs);
                }
                SchedulerOps::RoundRobin(rr) => {
                    rr.add_task(tid);
                        dbg!("Run Queue after adding {}: {:?}\n", tid, rr);
                }
            };
        }
    }


    // This task performs IPI message processing and dead task cleanup
    fn percpu_sched_worker(){
        let cpuid       = *(THIS_CPU_ID.borrow());
        let worker_tid  = *(CURR_TID.borrow());
        let mut last_pool_size;
        loop {
            let tpool = TASK_POOL.borrow();
            last_pool_size = tpool.len();
            let mut delete      = false;
            let mut tid_to_del  = 0;
            for (tid, task) in tpool.iter() {
                if task.state == TaskState::Terminating {
                    delete = true;
                    tid_to_del = *tid;
                    break;
                }
            }
            if delete == true {
                if tid_to_del == worker_tid {
                    dbg!("BUG: Deleting worker CPU:{} TID:{}\n",
                        cpuid, worker_tid);
                } else {
                    TASK_POOL.borrow_mut().remove(&tid_to_del);
                    dbg!("CPU({})_WORKER({}): Deleted TID {} - \n",
                        cpuid, worker_tid, tid_to_del);
                }
            }
            if TASK_POOL.borrow().len() == 1 && last_pool_size != 1 {
                klog!("<CPU {} IDLE - Free Frames: {}>", cpuid,
                        pmm_num_free_frames());
                loop {
                    // For some reason it ends up emptying the pool if we
                    // leave the interrupts enabled
                    arch::cpu_disable_ints();
                    arch::cpu_halt();
                    
                }
            }
            arch::cpu_busywait_us(100_000); // Implement Sleep
        }
    }
}

pub struct WaitChannel {
    waiters:    Spinlock<LinkedList<usize>>,
}
impl WaitChannel {
    pub const fn new() -> Self {
        Self {
            waiters: Spinlock::new(LinkedList::new())
        }
    }


    pub fn wait(&self) {
        if let Ok(_lock) = Preemption::lock() {
            let curr_tid = *(CURR_TID.borrow());
            {
                self.waiters.lock().push_back(curr_tid);
            }
            SCHEDULER.borrow_mut().block_task(curr_tid);
        }
    }

    pub fn wait_timeout(&self, _timeout: Option<SysTimerDuration>) {
        panic!("Not implemented - Implement sleep first")
    }

    // End the wait for up to n waiters
    pub fn signal(&self, n: usize) {
        let mut w = self.waiters.lock();
        for _i in 0..n {
            if let Some(tid) = w.pop_front() {
                SCHEDULER.borrow_mut().unblock_task(tid);
            }
        }
    }

    pub fn signal_all(&self) {
        let mut w = self.waiters.lock();
        while w.is_empty() == false {
            if let Some(tid) = w.pop_front() {
                SCHEDULER.borrow_mut().unblock_task(tid);
            } else {
                break;
            }
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
pub struct Preemption {
    interrupts_enabled: bool
}
impl Preemption {
    pub fn lock() -> Result<Self, ()> {
        let intf = arch::cpu_ints_enabled();
        arch::cpu_disable_ints();
        Ok(Self {interrupts_enabled: intf})
    }
}
impl Drop for Preemption {
    fn drop(&mut self) {
        if self.interrupts_enabled {
            arch::cpu_enable_ints();
        }
    }
}


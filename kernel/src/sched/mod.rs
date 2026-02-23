//
// BlightOS Kernel
//
// Task Scheduler
//   Provides an interface to the kernel code for switching between tasks and
//   address spaces based on the selected scheduler, and the target architecture
//
// TODO
//     Prevent (panic) when blocking a task that's holding a spinlock
//     Add a name field to each task/process
//     Support for mutexes
//     Support for RWLocks
use crate::arch::{self, SystemTimer, SystemTimerTrait};
use crate::arch::{cpu_count, cpu_ints_enabled, cpu_id};
use crate::mem::phys::*;
use crate::mem::virt::AddressSpace;
use crate::sched::sched_rr::FcfsScheduler;
use crate::sched::sched_rr::RoundRobinScheduler;
use crate::util::*;
use core::{fmt, usize};
use core::fmt::Display;
use core::slice::*;
use core::time::Duration;
use alloc::collections::BTreeMap;
use alloc::collections::LinkedList;
use alloc::format;
use alloc::string::String;
use core::panic;
use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};


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

static DEFAULT_STACK_SIZE:  AtomicUsize = AtomicUsize::new(PHY_FRAME_SIZE * 4);
static NEXT_TASK_ID:        AtomicUsize = AtomicUsize::new(0);
static MSG_QUEUE:           Spinlock<LinkedList<InterProcessorMessage>> = 
                                    Spinlock::new(LinkedList::new());
static TID_CPU_MAP:         Spinlock<BTreeMap<usize, usize>> = 
                                    Spinlock::new(BTreeMap::new());
static LOAD_BALANCER:       Spinlock<LoadBalancer> =
                                    Spinlock::new(LoadBalancer::new());
                            
// Operations that originate in one CPU but have to be executed on a different
// CPU will issue an IPM onto the shared MSG_QUEUE.
struct InterProcessorMessage {
    // The CPU whose worker has to receive & execute the message
    pub dest_cpu:   usize,

    pub msg :       InterProcessorMessagePayload
}
enum InterProcessorMessagePayload {
    // Some IPMs have to return a status code to the initiator task who are
    // blocked wait for the response. The destination CPU's worker performs the
    // request and sends a RespondAndUnblock back to the initator's CPU worker
    // who will update the blocked task with the mes_ret code and unblocks it.
    RespondAndUnblock {
        caller_tid: usize,
        msg_ret: usize
    },
    // Task caller_tid on the CPU caller_cpu has requested a new task to be
    // created on the dest_cpu. The dest_cpu's worker will create the task and
    // send a RespondAndUnblock to the initiator CPU's worker with the new tid.
    CreateTask {
        caller_tid: usize,
        caller_cpu: usize,
        func: fn(),
        stack_pgs:  usize,
        name:       String
    },
    // Migrates task from the CPU it's currently running on to dest_cpu
    MoveTask {
        task: Task
    },
    // Unblock task (tid) on the dest_cpu if it's blocked
    Unblock {
        tid: usize
    },
    // A task on different CPU wants to join() on the task dest_tid on the
    // CPU dest_cpu. If by the time the dest_cpu's worker receives this message
    // the dest_tid hasn't moved to a different CPU a RespondAndUnblock will
    // with msg_ret=dest_tid will be returned. Otherwise, the caller must either
    // send another AddJoiner to the new CPU, or poll the termination of the
    // dest_tid
    AddJoiner {
        dest_tid:   usize,
        joiner_tid: usize,
        joiner_cpu: usize,
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
#[repr(usize)]
pub enum TaskState {
    New,            // Struct allocated but not initalized
    Ready,          // Runnable - In a run queue but not running atm
    Running,        // Currently running task
    Blocked,        // Waiting for an event (join, wait channel signal, etc)
    Sleeping,       // Stays in the run-queue but gets skipped until it's time
    Migrating,      // Must be removed from its CPU pool and move to another
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
            TaskState::Sleeping     => write!(f, "Sleeping"),
            TaskState::Migrating    => write!(f, "Migrating"),
            TaskState::Terminating  => write!(f, "Terminating"),
            TaskState::Dropped      => write!(f, "Dropped"),
        }
        
    }
}

///
/// Generic Task Structure
/// 
#[derive(Debug)]
pub struct Task {
    tid:            usize, // Task ID
    pid:            usize, // Process Address Space ID (0: Kernel-space/no proc)
    name:           String,
    pub state:      TaskState,
    pub cpu:        u64, // Todo use as a mask, but for now run on 1 cpu only
    //
    stack_base:     usize,
    stack_pages:    usize,
    // Priority fields for RMS, FQS (b/p), etc
    _sched_p1:      u64,
    _sched_p2:      u64,
    _sched_p3:      u64,
    // Wake-up time in timestamp ticks if in the Sleeping state
    wakeup_time:    u64,
    // A reference counter for handling racing/multiple block/unblock calls
    blocked_count:  AtomicIsize,
    // List of tasks waiting/blocked on this one to finish
    joiners:        LinkedList<usize>,
    // CPU-dependent Runtime Context
    pub ctx:        arch::TaskContext,
    // Some IPM messages (e.g., CreateTask) return a code/result.
    pub msg_ret:    usize

}
impl Task {
    
    pub const fn new() -> Self {
        Self {
            tid:            0,
            pid:            0,
            name:           String::new(),
            state:          TaskState::New,
            ctx:            arch::TaskContext::new(),
            cpu:            0,
            stack_base:     0,
            stack_pages:    0,
            _sched_p1:      0,
            _sched_p2:      0,
            _sched_p3:      0,
            wakeup_time:    0,
            blocked_count:  AtomicIsize::new(0),
            joiners:        LinkedList::new(),
            msg_ret:        0
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

    pub fn current_tid() -> usize {
        *(CURR_TID.borrow())
    }

    pub fn current_pid() -> usize {
        let _ = Preemption::lock();
        let tid = Self::current_tid();
        let tpool = TASK_POOL.borrow_mut();
        let pid = tpool.get(&tid).expect("Task not found").pid;
        pid
    }

    pub fn name() -> String {
        let _ = Preemption::lock();
        let tid = Self::current_tid();
        let tpool = TASK_POOL.borrow_mut();
        let task = tpool.get(&tid).expect("Task not found");
        task.name.clone()
    }

    pub fn current_cpu() -> usize {
        cpu_id()
    }

    pub fn set_default_stack_size(size_bytes: usize) {
        DEFAULT_STACK_SIZE.store(size_bytes, Ordering::Relaxed);
    }

    //
    // Creates a task with the default stack size on a CPU chosen by the
    // load-balancer and returns the tid of the new task.
    //
    pub fn spawn(func: fn()) -> usize {
        Self::spawn_named(func, String::new())
    }

    pub fn spawn_named(func: fn(), name: String) -> usize {
        let target_cpu;
        {
            let lb = LOAD_BALANCER.lock();
            target_cpu = lb.select_cpu();
        }
        Self::spawn_on_cpu(func, target_cpu, name)
    }

    //
    // Creates a new task the specified CPU with the default stack size.
    // Returns the tid of the new task.
    // The caller will remain blocked until the target CPU's scheduler receives
    // the message, creates the task and returns the tid to the caller.
    //
    pub fn spawn_on_cpu(func: fn(), target_cpu: usize, name: String) -> usize{
        let tid = Self::current_tid();
        let cpu = Self::current_cpu();
        let stack_pgs = round_up!(DEFAULT_STACK_SIZE.load(Ordering::Relaxed),
                                    PHY_FRAME_SIZE) / PHY_FRAME_SIZE;
        let ret;
        if target_cpu == cpu {
            // Local task creation - no need for inter-processor messaging
            let sched = SCHEDULER.borrow_mut();
            ret = sched.create_task(func, stack_pgs, name)
        } else {
            // Remote task creation - Send and IPM and block
            Scheduler::send_ipm(
                InterProcessorMessage {
                    dest_cpu: target_cpu,
                    msg: InterProcessorMessagePayload::CreateTask {
                        caller_tid: tid,
                        caller_cpu: cpu,
                        func:       func,
                        stack_pgs:  stack_pgs,
                        name:       name
                    }
                }
            );
            Self::block();
            ret = Self::get_msg_ret(tid);
            Self::set_msg_ret(tid, 0); // Clear the ret value
        }
        ret
    }

    pub fn preempt(){
        arch::cpu_trigger_systimer_irq();
    }

    pub fn exists(tid: usize) -> bool {
        let tid_to_cpu = TID_CPU_MAP.lock();
        match tid_to_cpu.get(&tid) {
            Some(_)  => true,
            None        => false
        }
    }

    pub fn join(tid: usize) {
        let target_cpu;
        let cur_tid = Self::current_tid();
        let cur_cpu = Self::current_cpu();

        {
            let tid_to_cpu = TID_CPU_MAP.lock();
            match tid_to_cpu.get(&tid) {
                Some(tcpu)  => {target_cpu = *tcpu},
                None        => {
                    // The task must have been dropped already. No need to wait.
                    return;
                }
            }
        }
        if cur_cpu == target_cpu {
            let _ = Preemption::lock();
            let tpool = TASK_POOL.borrow_mut();
            match tpool.get_mut(&tid) {
                Some(target_task)   => {
                    target_task.joiners.push_back(cur_tid);
                    SCHEDULER.borrow_mut().block_task(cur_tid);
                    return;
                },
                None => {} // The task already exited or moved
            }
        }
        // Try to send an AddJoiner message to the CPU that hosts tid,
        // block and wait for the response. If the response == tid, we have
        // successfully added the current/calling thread to tid's queue and
        // we can block again for until tid terminates and unblock us.
        // Otherwise, tid must have moved to a new CPU, and we should retry
        // or resort to polling.
        {
            Scheduler::send_ipm(
                InterProcessorMessage {
                    dest_cpu: target_cpu,
                    msg: InterProcessorMessagePayload::AddJoiner {
                        dest_tid: tid,
                        joiner_tid: cur_tid,
                        joiner_cpu: cur_cpu
                    }
                }
            );
        }
        Self::block();
        let ret = Self::get_msg_ret(cur_tid);
        Self::set_msg_ret(cur_tid, 0);
        if ret != tid {
            // Poll for the termination of tid
            while Task::exists(tid) {
                Task::sleep(Duration::from_millis(200));
            }
        } else {
            // Successfully join tid's queue. Block here.
            Self::block();
            // The current tid will be unblocked when tid terminates
        }
    }

    pub fn block() {
        if cpu_ints_enabled() == false {
            // We are most probably executing a syscall.
            // TODO - Make sure the process is not holding spinlocks!!
            arch::cpu_enable_ints();
        }
        SCHEDULER.borrow_mut().block_task(Self::current_tid());
    }

    pub fn sleep(d: Duration){
        let tid = Self::current_tid();
        let tpool = TASK_POOL.borrow_mut();
        match tpool.get_mut(&tid) {
            Some(task)      => {
                task.wakeup_time    = SystemTimer::current_timestamp() +
                                    SystemTimer::duration_to_timestamp_ticks(d);
                task.state          = TaskState::Sleeping;
            }
            None            => {}
        }
        Task::preempt();
    }

    // Unblocks a task waiting on a wait channel or sleeping
    pub fn wake(tid: usize) {
        SCHEDULER.borrow_mut().unblock_task(tid);
    }

    pub fn exit() {
        SCHEDULER.borrow_mut().terminate_task();
    }

    //
    // Moves the current task to a different CPU
    //
    pub fn migrate_to_cpu(new_cpu: usize) {
        let cpuid = cpu_id();
        if new_cpu == cpuid || new_cpu >= cpu_count() {
            return; // Nothing to do
        }
        // Copy and remove this task from the current pool/runqueue and put it
        // on the messge queue of the destination CPU
        let _ = Preemption::lock();
        let tid = *(CURR_TID.borrow());
        let tpool = TASK_POOL.borrow_mut();
        match tpool.get_mut(&tid) {
            Some(task)      => {
                task.cpu    = new_cpu as u64;
                task.state  = TaskState::Migrating
            }
            None            => {}
        }
        SCHEDULER.borrow_mut().block_task(tid);
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

    // Private interface
    fn get_msg_ret(tid: usize) -> usize {
        let ret = TASK_POOL.borrow().get(&tid)
                                    .expect("Task not found!").msg_ret;
        ret
    }

    fn set_msg_ret(tid: usize, val: usize) {
        TASK_POOL.borrow_mut().get_mut(&tid)
                                .expect("Task not found!").msg_ret = val;
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if self.state != TaskState::Terminating {
            klog!("BUG: Drop called on a non-Terminating Task: {}({:?})\n",
                self.tid, self.state);
            return;
        }
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

///
/// Generic Task Scheduler
///
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
    //
    // Class methods
    //
    pub const fn new() -> Self {
        Self {
            ops: SchedulerOps::FirstComeFirstServe(FcfsScheduler::new())
        }
    }
    // Policy selection routines
    pub fn config_first_come_first_served() {
        let sched = SCHEDULER.borrow_mut();
        sched.ops = SchedulerOps::FirstComeFirstServe(FcfsScheduler::new());
        let mut lb = LOAD_BALANCER.lock();
        lb.add_cpu(Task::current_cpu());
    }

    pub fn config_round_robin(quantum: Duration) {
        let sched = SCHEDULER.borrow_mut();
        sched.ops = SchedulerOps::RoundRobin(RoundRobinScheduler::new());
        if let SchedulerOps::RoundRobin(rr) = &mut sched.ops {
            rr.init(quantum);
        }
        let mut lb = LOAD_BALANCER.lock();
        lb.add_cpu(Task::current_cpu());
    }

    //
    // Instance Methods
    //

    // Start the scheduler on the current CPU starting with the scheduler's
    // worker task for the CPU as the first task
    // Swithing to the first tast will also enable interrupts on the CPU
    pub fn start_scheduling(&mut self) {
        let cpuid = cpu_id();
        // Add the percpu worker scheduler task
        let pcpuw_tid = self.create_task(Self::percpu_sched_worker, 2,
                            format!("CPU{}-WORKER", cpuid));
        let pcpuw_task = TASK_POOL.borrow_mut().get_mut(&pcpuw_tid)
                .expect("Cannot start the scheduler without a starting task");
        CURR_TID.write(pcpuw_tid);
        pcpuw_task.state = TaskState::Running;
        dbg!("Starting on CPU {} w\\ PCPUWorkerTID: {}(s:{})\n",
            cpuid, pcpuw_tid, pcpuw_task.state);
        arch::cpu_switch_context_nosave(&pcpuw_task.ctx);
    }

    // Returns the tid of the task created
    pub fn create_task(&mut self, func: fn(), stack_pgs: usize, name: String) -> 
    usize {
        let _ = Preemption::lock();
        // Changing the per-cpu pool -> Disable Preemption
        if let Some(stack) = self.stack_alloc(stack_pgs) {
            let cpuid = cpu_id();
            let tid   = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
            let tpool = TASK_POOL.borrow_mut();
            let new_task;
            tpool.insert(tid, Task::new());
            new_task = tpool.get_mut(&tid).expect("Task pool failure");
            new_task.tid            = tid;
            new_task.cpu            = cpuid as u64;
            new_task.name           = name;
            new_task.stack_base     = stack.as_mut_ptr() as usize;
            new_task.stack_pages    = stack.len() * 
                                            size_of::<usize>() / PHY_FRAME_SIZE;
            new_task.ctx.init(tid, func, stack);
            {
                let mut tid_cpu_map = TID_CPU_MAP.lock();
                tid_cpu_map.insert(tid, cpuid);
            }
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
            let mut lb = LOAD_BALANCER.lock();
            lb.add_task(Task::current_cpu());
            return tid;
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
            {
                let mut lb = LOAD_BALANCER.lock();
                lb.rem_task(Task::current_cpu());
            }
            TASK_POOL.borrow_mut().get_mut(&term_tid)
                .unwrap().state = TaskState::Terminating;
            arch::cpu_trigger_systimer_irq();
        }
    }

    // Should be only called in an irq context
    pub fn preempt_irq(&mut self) {
        let cur_task = *(CURR_TID.borrow()); // Put the outgoing TID on the stack
        let mut next_task;

        // Run queues only hold ready/running/sleeping tasks. Skip 
        loop {
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
                    if next_task.state == TaskState::Sleeping {
                        if SystemTimer::current_timestamp() >=
                                                    next_task.wakeup_time {
                            next_task.state = TaskState::Ready;
                            next_task.wakeup_time = 0;
                            break;
                        }
                        // Sleeping task not ready to wake up. Skip to the next
                    } else {
                        // Non-sleeping task selected.
                        break;
                    }
                }
                None        => {
                    panic!("The run queue should never get empty!");
                }
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
        let _ = Preemption::lock();
        dbg!("-BLOCKING TID {}\n", tid);
        let task = TASK_POOL.borrow_mut().get_mut(&tid)
                                                .expect("Task not found");
        
        if task.blocked_count.fetch_add(1, Ordering::Relaxed) != 0 {
            // No need to block - There are pending/early unblock calls
            return;
        }

        if task.state != TaskState::Migrating {
            task.state = TaskState::Blocked;
        }

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

    pub fn unblock_task(&mut self, tid: usize) {
        let target_cpu;
        {
            let tid_to_cpu = TID_CPU_MAP.lock();
            match tid_to_cpu.get(&tid) {
                Some(tcpu)  => {target_cpu = *tcpu},
                None        => {
                    // The task must have been dropped already.
                    return;
                }
            }
        }

        let _ = Preemption::lock();
        if Task::current_cpu() == target_cpu {
            // tid on the same CPU, just add it back to the run queue
            dbg!("+UNBLOCKING TID {}\n", tid);
            let task = TASK_POOL.borrow_mut().get_mut(&tid)
                                                    .expect("Task not found");
            if task.state == TaskState::Sleeping {
                // Wake the task up - It's already in the run queue!
                task.state = TaskState::Ready;
                task.wakeup_time = 0;
            } else if task.blocked_count.fetch_sub(1, Ordering::Relaxed) == 1 {
                // Unblock the task when undoing the last block call
                task.state = TaskState::Ready;
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
        } else {
            // tid on a different CPU. Send and IPM
            Scheduler::send_ipm(
                InterProcessorMessage {
                    dest_cpu: target_cpu as usize,
                    msg: InterProcessorMessagePayload::Unblock {
                        tid: tid 
                    }
                }
            );
        }
    }

    //
    // Class Methods - continued
    //
    // This task performs IPI message processing, dead task cleanup, etc
    fn percpu_sched_worker(){
        let cpuid       = cpu_id();
        let worker_tid  = *(CURR_TID.borrow());
        let mut last_pool_size;
        dbg!("{} started on cpu {}\n", Task::name(), cpuid);
        loop {
            // 1) Process one state-related action
            let mut target_tid  = 0;
            let mut delete      = false;
            let mut move_out    = false;
            {
                let tpool = TASK_POOL.borrow();
                last_pool_size = tpool.len();
                for (tid, task) in tpool.iter() {
                    match task.state {
                        TaskState::Terminating  => {
                            delete = true;
                            target_tid = *tid;
                            break;
                        },
                        TaskState::Migrating    => {
                            move_out = true;
                            target_tid = *tid;
                        },
                        _                       => {}
                    }
                }
            }
            if delete == true {
                if target_tid == worker_tid {
                    klog!("BUG: Deleting worker CPU:{} TID:{}\n",
                        cpuid, worker_tid);
                } else {
                    TASK_POOL.borrow_mut().remove(&target_tid);
                    let mut tid_cpu_map = TID_CPU_MAP.lock();
                    tid_cpu_map.remove(&target_tid);
                    dbg!("CPU({})_WORKER({}): Deleted TID {} - \n",
                        cpuid, worker_tid, target_tid);
                }
            } else if move_out == true {
                let mut target_task = TASK_POOL.borrow_mut().remove(&target_tid)
                                                .expect("Task not found");
                {
                    let mut lb = LOAD_BALANCER.lock();
                    lb.rem_task(Task::current_cpu());
                }
                target_task.state = TaskState::Blocked;
                Self::send_ipm(
                    InterProcessorMessage {
                        dest_cpu: target_task.cpu as usize,
                        msg: InterProcessorMessagePayload::MoveTask {
                            task: target_task
                        },
                    }
                );
                dbg!("CPU({})_WORKER({}): Migrated TID {} Out\n",
                            cpuid, worker_tid, target_tid);
            }

            // 2) Check for inter-processor messages left for this CPU
            let mut msgs;
            {
                let mut mq = MSG_QUEUE.lock();
                msgs = mq.extract_if(|msg| msg.dest_cpu == cpuid)
                                            .collect::<LinkedList<_>>();
            }
            while msgs.is_empty() == false {
                let ipm = msgs.pop_front().unwrap();
                Self::process_ipm(ipm, cpuid, worker_tid);
            }

            // No more tasks to run - Go into idle mode
            if TASK_POOL.borrow().len() == 1 {
                if last_pool_size != 1 {
                    // Log in the iteration when the last task gets dropped
                    if cpuid == 0 {
                        klog!("<{} - Free Frames: {}>", Task::name(),
                                pmm_num_free_frames());
                    } else {
                        dbg!("<{} IDLE>\n", Task::name());
                    }
                    
                }

            }
            // Task::preempt();
            // Interrupts are enabled and a timer interrupt will break
            // out of cpu_hlt/idle mode
            arch::cpu_enable_ints(); // To prevent bricking the core
            arch::cpu_halt();
        }
    }

    fn send_ipm(ipm: InterProcessorMessage) {
        let mut mq = MSG_QUEUE.lock();
        mq.push_back(ipm);
    }

    fn process_ipm(ipm: InterProcessorMessage, cpuid: usize, _wtid: usize) {
        match ipm.msg {
            InterProcessorMessagePayload::MoveTask { task } => {
                let new_tid = task.tid;
                TASK_POOL.borrow_mut().insert(task.tid, task);
                {
                    let mut tid_cpu_map = TID_CPU_MAP.lock();
                    *(tid_cpu_map.get_mut(&new_tid).unwrap()) = cpuid;
                    let mut lb = LOAD_BALANCER.lock();
                    lb.add_task(Task::current_cpu());
                }
                SCHEDULER.borrow_mut().unblock_task(new_tid);
                dbg!("CPU({})_WORKER({}): Migrated TID {} In\n",
                        cpuid, _wtid, new_tid);
            },
            InterProcessorMessagePayload::Unblock { tid }   => {
                SCHEDULER.borrow_mut().unblock_task(tid);
                dbg!("CPU({})_WORKER({}) Unblocked TID {}\n",
                        cpuid, _wtid, tid);
            },
            InterProcessorMessagePayload::CreateTask 
            { caller_tid, caller_cpu, func, stack_pgs, name} => {
                dbg!("IPM::CreateTask received by {}\n", Task::name());
                let new_tid = SCHEDULER.borrow_mut()
                                .create_task(func, stack_pgs, name);
                Self::send_ipm(
                    InterProcessorMessage {
                        dest_cpu: caller_cpu,
                        msg: InterProcessorMessagePayload::RespondAndUnblock {
                            caller_tid: caller_tid,
                            msg_ret:    new_tid
                        }
                    }
                );
            },
            InterProcessorMessagePayload::RespondAndUnblock 
            { caller_tid, msg_ret }                     => {
                Task::set_msg_ret(caller_tid, msg_ret);
                SCHEDULER.borrow_mut().unblock_task(caller_tid);
            },
            InterProcessorMessagePayload::AddJoiner
            { dest_tid, joiner_tid, joiner_cpu }        => {
                let _ = Preemption::lock();
                let tpool = TASK_POOL.borrow_mut();
                match tpool.get_mut(&dest_tid) {
                    Some(target_task)   => {
                        // Add the joiner to target's queue
                        target_task.joiners.push_back(joiner_tid);
                        // Send a response back to joiner's CPU
                        Self::send_ipm(
                            InterProcessorMessage {
                                dest_cpu: joiner_cpu,
                                msg: InterProcessorMessagePayload::
                                                            RespondAndUnblock{
                                    caller_tid: joiner_tid,
                                    msg_ret:    dest_tid
                                }
                            }
                        );
                    }
                    None => {
                        // Target task already exited or moved
                        // Send a failure response back to joiner's CPU
                        Self::send_ipm(
                            InterProcessorMessage {
                                dest_cpu: joiner_cpu,
                                msg: InterProcessorMessagePayload::
                                                            RespondAndUnblock{
                                    caller_tid: joiner_tid,
                                    msg_ret:    0
                                }
                            }
                        );
                    }
                }
            }
        }
    }
}

///
/// Basic Load Balancer
///
struct CPULoad {
    cpuid:      usize,
    num_tasks:  usize,
}
struct LoadBalancer {
    cpu_vector: LinkedList<CPULoad>
}
impl LoadBalancer {
    pub const fn new() -> Self {
        Self {
            cpu_vector:     LinkedList::new()
        }
    }

    pub fn add_cpu(&mut self, cpuid: usize) {
        self.cpu_vector.push_back(CPULoad { cpuid, num_tasks: 0 });
    }

    pub fn add_task(&mut self, cpuid: usize) {
        let opt = self.cpu_vector.iter_mut().find(|item| item.cpuid == cpuid);
        if let Some(cpuload) = opt {
            cpuload.num_tasks += 1;
        }
    }

    pub fn rem_task(&mut self, cpuid: usize) {
        let opt = self.cpu_vector.iter_mut().find(|item| item.cpuid == cpuid);
        if let Some(cpuload) = opt {
            cpuload.num_tasks -= 1;
        }
    }

    // Returns the id of CPU where the next task should be added to, i.e.,
    // the CPU with the least # of tasks in this implementation
    pub fn select_cpu(&self) -> usize {
        let mut least_utilization = usize::MAX;
        let mut selected_cpu = 0;
        for cpuload in self.cpu_vector.iter() {
            if cpuload.num_tasks < least_utilization {
                least_utilization = cpuload.num_tasks;
                selected_cpu = cpuload.cpuid;
            }
        }
        selected_cpu
    }
}

///
/// Wait Channel
/// 
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

    pub fn wait_timeout(&self, _timeout: Option<Duration>) {
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


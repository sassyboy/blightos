//
// BlightOS Kernel
//
// Self-test
//

use core::hint::spin_loop;
use core::sync::atomic::*;
use core::time::Duration;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::util::*;
use crate::arch::*;
use crate::drivers::storage::*;
use crate::mem::phys::{pfree, pmm_num_free_frames};
use crate::sched::{Task, WaitChannel};
use crate::fs::{DirectoryEntry, MountPoint};
use crate::drivers::storage::IOCompletion;
use crate::drivers::input::{Keyboard, KeyboardEvent};


static BSP_T1_TID: AtomicUsize = AtomicUsize::new(0);
static BSP_T2_TID: AtomicUsize = AtomicUsize::new(0);
static BSP_T3_TID: AtomicUsize = AtomicUsize::new(0);
static TEST_TASKS_STARTED: AtomicUsize = AtomicUsize::new(0);
static SHARED_VAR : Spinlock<i32> = Spinlock::new(0);

pub fn kself_test() {
    // This routine can be called multiple times from the user-space
    // Reinitialize everything
    TEST_TASKS_STARTED.store(0, Ordering::Relaxed);
    {
        let mut shared_var = SHARED_VAR.lock();
        *shared_var = 0;
    }
    
    crate::kearly_console::init();
    klog!("[KERNEL SELF-TEST]: Starting...\n");
    klog!("[TEST] Launching 2 tasks per CPU to compete over the same \
            counter and screen buffer\n");
    for c in 0..cpu_count() {
        if c == 0 {
            let t = Task::spawn_on_cpu(task1_exec, 0, 0, format!("Cpu0Tst1.1"));
            BSP_T1_TID.store(t, Ordering::Relaxed);
            let t = Task::spawn_on_cpu(task2_exec, 0, 0, format!("Cpu0Tst1.2"));
            BSP_T2_TID.store(t, Ordering::Relaxed);
        } else {
            Task::spawn_on_cpu(task1_exec, 0, c, format!("Cpu{}-Tst1.1", c));
            Task::spawn_on_cpu(task2_exec, 0, c, format!("Cpu{}-Tst1.2", c));
        }
    }
    while BSP_T3_TID.load(Ordering::Relaxed) == 0 {
        Task::preempt();
    }
    Task::join(BSP_T3_TID.load(Ordering::Relaxed));

}

fn task1_exec(_arg: usize) {
    let cpuid = Task::current_cpu();
    // Wait for all the test tasks to spawn before racing over the counter
    TEST_TASKS_STARTED.fetch_add(1, Ordering::Relaxed);
    while TEST_TASKS_STARTED.load(Ordering::Relaxed) < cpu_count() * 2 {
        spin_loop();
    }
    loop {
        {
            let mut shared_var = SHARED_VAR.lock();
            if *shared_var >= 100 {
                break;
            }
            *shared_var += 1;
            klog!("<C{}/T{}={}>", cpuid, Task::current_tid(), *shared_var);
        }
        Task::sleep(Duration::from_millis(5));
    }
}

fn task2_exec(_arg: usize) {
    let cpuid = Task::current_cpu();
    // Wait for all the test tasks to spawn before racing over the counter
    TEST_TASKS_STARTED.fetch_add(1, Ordering::Relaxed);
    while TEST_TASKS_STARTED.load(Ordering::Relaxed) < cpu_count() * 2 {
        spin_loop();
    }
    if cpuid == 0 {
        let tid = Task::spawn_named(task3_exec, 0, String::from("KSelfTest"));
        BSP_T3_TID.store(tid, Ordering::Relaxed); 
    }
    loop {
        {
            let mut shared_var = SHARED_VAR.lock();
            if *shared_var >= 100 {
                break;
            }
            *shared_var += 1;
            klog!("<C{}/T{}={}>", cpuid, Task::current_tid(), *shared_var);
        }
        Task::sleep(Duration::from_millis(5));
    }
}

static WC : WaitChannel = WaitChannel::new();

fn task3_exec(_arg: usize) {
    klog!("<A LONG MESSAGE TO TEST THE CONSOLE PRINT_STR LOCK - {} (TID: {})>",
                Task::name(), Task::current_tid());
    // Wait for the first two tasks to finish
    Task::join(BSP_T1_TID.load(Ordering::Relaxed));
    Task::join(BSP_T2_TID.load(Ordering::Relaxed));
    Task::sleep(Duration::from_secs(1)); // Wait for task on other CPUs
    klog!("\n");
    // SLEEP IMPLEMENTATION ----------------------------------------------------
    klog!("[Test] Best-effort Sleep\n");
    let tsc0 = SystemTimer::current_timestamp_as_duration();
    Task::sleep(Duration::from_millis(5));
    let tsc1 = SystemTimer::current_timestamp_as_duration();
    Task::sleep(Duration::from_millis(15));
    let tsc2 = SystemTimer::current_timestamp_as_duration();
    klog!("  Start ts: {} us, +5ms sleep: {} us, +15ms sleep: {} us - \
            deltas: {} us, {} us \n",
        tsc0.as_micros(), tsc1.as_micros(), tsc2.as_micros(),
        tsc1.as_micros() - tsc0.as_micros() - 5000,
        tsc2.as_micros() - tsc1.as_micros() - 15000
    );
    klog!("[Test] Best-effort Sleep + 5 other tasks running\n  ");
    for i in 0..5 {
        Task::spawn_on_cpu(|_arg: usize| {
            klog!("<{}>", Task::name());
            cpu_busywait(Duration::from_millis(50));
        }, Task::current_cpu(), 0, format!("ST{}", i));
    }
    let tsc0 = SystemTimer::current_timestamp_as_duration();
    Task::sleep(Duration::from_millis(5));
    let tsc1 = SystemTimer::current_timestamp_as_duration();
    Task::sleep(Duration::from_millis(15));
    let tsc2 = SystemTimer::current_timestamp_as_duration();
    klog!("\n  Start ts: {} us, +5ms sleep: {} us, +15ms sleep: {} us - \
            deltas: {} us, {} us \n",
        tsc0.as_micros(), tsc1.as_micros(), tsc2.as_micros(),
        tsc1.as_micros() - tsc0.as_micros() - 5000,
        tsc2.as_micros() - tsc1.as_micros() - 15000
    );
    klog!("[Test] Waking up a asleep task early\n");
    let sleeping_tid = Task::spawn( |_arg: usize| {
        Task::sleep(Duration::from_secs(20));
    }, 0);
    Task::sleep(Duration::from_millis(100));
    Task::wake(sleeping_tid);
    Task::join(sleeping_tid);

    // TEST - PARALLEL HEAP ALLOCATION -----------------------------------------
    klog!("[Test] Parallel heap allocations - Free frames: {}\n",
            pmm_num_free_frames());

    let t4 = Task::spawn(|_arg: usize| {
        klog!("  <Task {}(CPU{}) allocate/verify/free 1000 i32>\n",
                Task::current_tid(), Task::current_cpu());
        let mut myvec: Vec<i32> = Vec::new();
        for i in 0..1000 {
            myvec.push(i);
        }
        Task::sleep(Duration::from_millis(20));
        for i in 0..1000 {
            if myvec[i] != i as i32 {
                klog!("  [FAIL] Vector element {} corrupted!\n", i);
                break;
            }
        }
        klog!("  <Task {} Finished>\n", Task::current_tid());
    }, 0);
    
    {
        Task::sleep(Duration::from_millis(5));
        let _myvar1: Box<usize> = Box::new(1234);
        let _myvar2: Box<usize> = Box::new(2341);
        let _myvar3: Box<usize> = Box::new(3412);
        let _myvar4: Box<usize> = Box::new(4123);
        let mut myvec: Vec<i32> = Vec::new();
        klog!("  <Task {}(CPU{}) allocate/verify/free 1000 i32>\n",
                Task::current_tid(), Task::current_cpu());
        for i in 0..1000 {
            myvec.push(i);
        }
        klog!("  [MIDPOINT] Free frames: {}\n", pmm_num_free_frames());
        klog!("  _myvars: {}, {}, {}, {}\n",
                    *_myvar1, *_myvar2, *_myvar3, *_myvar4);
        for i in 0..1000 {
            if myvec[i] != i as i32 {
                klog!("  [FAIL] Vector element {} corrupted!\n", i);
                break;
            }
        }
    }
    Task::join(t4);
    klog!("  Free frames: {}, Cached TLSF Metadata: ~5\n",
        pmm_num_free_frames());

    // TEST - CO-OP SCHEDULING -------------------------------------------------
    klog!("[TEST] Co-op scheduling\n  ");
    let t5 = Task::spawn_on_cpu(|_arg: usize| {
        for _i in 0..10 {
            klog!("<{}>", Task::name());
            Task::preempt();
            
        }
    }, 0, Task::current_cpu(), format!("COP2"));

    for _i in 0..10 {
        Task::preempt();
        klog!("<COP1>");
        
    }
    Task::join(t5);
    klog!("\n");

    // TEST - REMOTE TASK CREATION AND JOIN ------------------------------------
    klog!("[Test] Remote task creation/join - Caller: tid={}, cpu={}\n",
                Task::current_tid(), Task::current_cpu());
    let tid = Task::spawn_on_cpu(|_arg: usize| {
        klog!("  Task {} currently running on CPU{}\n",
                                    Task::current_tid(), Task::current_cpu());
        Task::sleep(Duration::from_millis(500));
    }, 0, (cpu_id() + 1) % cpu_count(), format!("RemoteCreateTestTask"));
    klog!("  Task {} joining the remote task {}\n", Task::current_tid(), tid);
    Task::join(tid);

    // TEST - TASK MIGRATION ---------------------------------------------------
    klog!("[TEST] Task migration\n");
    let tid = Task::spawn(|_arg: usize|
    {
       
        klog!("  Task {} currently running on CPU{}\n",
                                    Task::current_tid(), Task::current_cpu());
        Task::migrate_to_cpu((Task::current_cpu() + 1) % cpu_count());
        klog!("  Task {} migrated and is currently running on CPU{}\n",
                                    Task::current_tid(), Task::current_cpu());
        Task::migrate_to_cpu((Task::current_cpu() + 1) % cpu_count());
        klog!("  Task {} migrated and is currently running on CPU{}\n",
                                    Task::current_tid(), Task::current_cpu());
        Task::migrate_to_cpu((Task::current_cpu() + 1) % cpu_count());
        klog!("  Task {} migrated and is currently running on CPU{}\n",
                                    Task::current_tid(), Task::current_cpu());
    }, 0);
    Task::join(tid);

    // TEST - SHARED WAIT CHANNEL ----------------------------------------------
    klog!("[TEST] Shared wait channel...\n  ");
    let mut wtid : [usize; 5] = [0; 5];
    for i in 0..5 {
        wtid[i] = Task::spawn(|_arg: usize| {
            klog!("<T{} Waiting on CPU {}>", 
                    Task::current_tid(), Task::current_cpu());
            
            WC.wait();
            klog!("<T{} Resumed on CPU {}>",
                    Task::current_tid(), Task::current_cpu());
        }, 0);
    }
    Task::sleep(Duration::from_millis(1000));
    klog!("\n  <Task {} Signaling all the waiters>\n  ", Task::current_tid());
    WC.signal_all();
    for i in 0..5 {
        if wtid[i] > 0 {
            Task::join(wtid[i]);
        }
    }

    klog!("\nPress any key to contiue the testing...\n");
    Keyboard::wait_for_event(KeyboardEvent::KeyReleased);
    crate::kearly_console::init();
    // TEST - DISK OPERATIONS --------------------------------------------------
    klog!("\n[TEST] Disk Drive Testing... Detected disks are:\n");
    {
        let ndisks = num_disks();
        for i in 0..ndisks {
            let dsk = get_disk(i).expect("Couldn't find the disk!");
            klog!("  {:X?}\n", dsk);
            let buf = crate::mem::phys::palloc().unwrap();
            let bufp: *mut u8 = buf as *mut u8;
            unsafe {
                bufp.write_bytes(0xAA, 4096);
            }

            for i in (0..10).rev() {
                let mut ioreq = IORequest::new();
                ioreq.req_id =          0;
                ioreq.sync =            true;
                ioreq.op =              IOOperation::Read;
                ioreq.lba =             1 + i; // GPT
                ioreq.sectors =         2;
                ioreq.buffer =          buf;
                // ioreq.completion_cb =   comp_cb;
                // (dsk.issue_io)(&dsk, &ioreq);
                let ret = submit_sync_io(&dsk, &mut ioreq);
                let ts_handled = SystemTimer::current_timestamp();
                match ret {
                    Some(comp_req) => {
                        match comp_req.completion_code {
                            IOCompletion::Successful(len)   => {
                                klog!("IoReq {} Successful: LBA:{:02}, bytes {}, \
                                    T: issue={:.2}, submit={:.2}, comp={:.2}, \
                                    handled={:.2}, Total:{:06.2}ms\n",
                                    comp_req.req_id, comp_req.lba ,len,
                                    SystemTimer::timestamp_to_duration(
                                            comp_req.ts_issued).as_micros()as f64 / 1000.0, 
                                    SystemTimer::timestamp_to_duration(
                                            comp_req.ts_submitted).as_micros() as f64 / 1000.0, 
                                    SystemTimer::timestamp_to_duration(
                                            comp_req.ts_completed).as_micros() as f64 / 1000.0, 
                                    SystemTimer::timestamp_to_duration(
                                            ts_handled).as_micros() as f64 / 1000.0,
                                    SystemTimer::timestamp_to_duration(
                                            ts_handled - comp_req.ts_issued).as_micros() as f64 / 1000.0
                                );
                            },
                            _   => {
                                klog!("FAILED IoReq: {:?}\n", comp_req);
                            }
                        }
                    },
                    None => {
                        klog!("submit_sync_io failed: {:?}\n", ioreq);
                    }
                }
            }
            dump_memory_columns(buf, 20 , 5);       
            pfree(buf);
        }
    }
    // List available mount points
    // Always return the disk%d.%d resources back
    klog!("Available mount-points: {:?}\n", MountPoint::list_names());
    // Root directory of disk0.0
    if let Some(mnt) = MountPoint::from_path("disk0.0:/") {
        klog!("disk0.0: Root directory content:\n");
        if let IOCompletion::Successful(hnd) = mnt.fopen("disk0.0:/") {
            let mut out_vec: Vec<DirectoryEntry> = Vec::new();
            let ioc = mnt.fenum(hnd, &mut out_vec);
            if let IOCompletion::Successful(_cnt) = ioc {
                for item in out_vec {
                    klog!("{}, {}, 0x{:X}\n", item.name, item.size, item.flags);
                }  
            }
        }
    }
    // FINISHED TESTING --------------------------------------------------------
    klog!("\n[KERNEL SELF-TEST] Finished - Free frames: {}\n",
        pmm_num_free_frames());
}
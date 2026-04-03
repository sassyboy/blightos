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
use crate::mem::heap::Kalloc;
use crate::util::*;
use crate::arch::*;
use crate::drivers::storage::*;
use crate::mem::phys::{pfree, pmm_num_free_frames};
use crate::sched::{Task, WaitChannel};
use crate::fs::{MountPoint, File};
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
    klog!("[Test] Parallel heap allocations - Free frames: {}\n\
           Pages used by the heap: Meta-data {}, User-data: {}\n",
            pmm_num_free_frames(),
            Kalloc::metadata_pages_used(), Kalloc::userdata_pages_used());

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
        klog!("  [MIDPOINT] Free frames: {}, \
                 Pages used by the heap: Meta-data {}, User-data: {}\n",
            pmm_num_free_frames(),
            Kalloc::metadata_pages_used(), Kalloc::userdata_pages_used());
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
    klog!("  Free frames: {}, \
             Pages used by the heap: Meta-data {}, User-data: {}\n",
            pmm_num_free_frames(),
            Kalloc::metadata_pages_used(), Kalloc::userdata_pages_used());

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
                        match comp_req.completion {
                            Ok(len)   => {
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
                            Err(_e)   => {
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
    match File::open("disk0.0:/", File::MODE_READ) {
        Ok(root_dir) => {
            klog!("disk0.0: Root directory content:\n");
            match root_dir.enumerate() {
                Ok(entries) => {
                    for entry in entries {
                        klog!("  {}, {}, 0x{:X}\n",
                            entry.name, entry.size, entry.flags);
                    }
                },
                Err(e) => {
                    klog!("Failed to enumerate disk0.0:/ - {:?}\n", e);
                }
            }
        },
        Err(e) => {
            klog!("Failed to open disk0.0:/ - {:?}\n", e);
        }
    }
    // FINISHED TESTING --------------------------------------------------------
    klog!("\n[KERNEL SELF-TEST] Finished - Free frames: {}\n",
        pmm_num_free_frames());
}

//
// Kernel heap correctness/performance testing commented out for efficiency
//
// static mut HEAP_TEST_POINTERS: [*mut usize; 64*255*3] = 
//             [core::ptr::null_mut(); 64*255*3];
// pub unsafe fn heap_correctness_test() {
//     klog!("[HEAP CORRECTNESS TEST] Allocating 64*255*3 64-byte variables\n");
//     // Do not store every timing sample — keep min/max/sum and counts only.
//     let total: usize = 64 * 255 * 3;
//     let mut alloc_min: u128 = u128::MAX;
//     let mut alloc_max: u128 = 0;
//     let mut alloc_sum: u128 = 0;
//     let mut alloc_count: usize = 0;
//     let mut alloc_hist: [usize; 11] = [0; 11]; // Histogram of allocation times (10 buckets)
//     let bucket_size: u128 = 1000; // 1 microsecond buckets

//     let mut free_min: u128 = u128::MAX;
//     let mut free_max: u128 = 0;
//     let mut free_sum: u128 = 0;
//     let mut free_count: usize = 0;
//     let mut free_hist: [usize; 11] = [0; 11]; // Histogram of free times (10 buckets)
//     let free_bucket_size: u128 = 1000; // 1 microsecond buckets

//     klog!("  [Before Alloc] Free frames: {}, \
//              Pages used by the heap: Meta-data {}, User-data: {}\n",
//             pmm_num_free_frames(),
//             Kalloc::metadata_pages_used(), Kalloc::userdata_pages_used());

//     // Allocating more than 255 clusters worth of AUs to test the chaining of
//     // descriptor pages. Time each allocation but only aggregate stats.
//     for i in 0..total {
//         let t0 = SystemTimer::current_timestamp();
//         let ptr: Box<usize> = Box::new(i);
//         let t1 = SystemTimer::current_timestamp();
//         let dt = SystemTimer::timestamp_to_duration(t1 - t0).as_nanos();

//         // Update histogram
//         let bucket = (dt / bucket_size) as usize;
//         if bucket < 10 {
//             alloc_hist[bucket] += 1;
//         } else {
//             alloc_hist[10] += 1; // Overflow bucket
//         }
//         // update min/max/sum/count
//         if dt < alloc_min { alloc_min = dt; }
//         if dt > alloc_max { alloc_max = dt; }
//         alloc_sum += dt;
//         alloc_count += 1;

//         let ptrp: *mut usize = Box::into_raw(ptr);
//         HEAP_TEST_POINTERS[i] = ptrp;
//     }

//     klog!("  [After Alloc] Free frames: {}, \
//              Pages used by the heap: Meta-data {}, User-data: {}\n",
//             pmm_num_free_frames(),
//             Kalloc::metadata_pages_used(), Kalloc::userdata_pages_used());

//     // Verify the integrity of the allocated data.
//     for i in 0..total {
//         let ptr = HEAP_TEST_POINTERS[i];
//         if ptr.is_null() || *ptr != i {
//             klog!("  [FAIL] Heap corruption at index {}: expected {}, got {}\n",
//                     i, i, if ptr.is_null() { usize::MAX } else { *ptr });
//             break;
//         }
//     }

//     // Free the allocated memory, timing each free but only aggregate stats.
//     for i in 0..total {
//         let ptrp = HEAP_TEST_POINTERS[i];
//         if !ptrp.is_null() {
//             let t0 = SystemTimer::current_timestamp();
//             // Recreate the Box and drop it to free.
//             let bx = Box::from_raw(ptrp);
//             drop(bx);
//             let t1 = SystemTimer::current_timestamp();
//             let dt = SystemTimer::timestamp_to_duration(t1 - t0).as_nanos();

//             // Update histogram
//             let bucket = (dt / free_bucket_size) as usize;
//             if bucket < 10 {
//                 free_hist[bucket] += 1;
//             } else {
//                 free_hist[10] += 1; // Overflow bucket
//             }
//             // update min/max/sum/count
//             if dt < free_min { free_min = dt; }
//             if dt > free_max { free_max = dt; }
//             free_sum += dt;
//             free_count += 1;

//             HEAP_TEST_POINTERS[i] = core::ptr::null_mut();
//         }
//     }

//     klog!("  [After Free] Free frames: {}, \
//              Pages used by the heap: Meta-data {}, User-data: {}\n",
//             pmm_num_free_frames(),
//             Kalloc::metadata_pages_used(), Kalloc::userdata_pages_used());

//     // Compute stats
//     let (alloc_min, alloc_max, alloc_avg) = if alloc_count > 0 {
//         (alloc_min, alloc_max, alloc_sum / (alloc_count as u128))
//     } else { (0, 0, 0) };

//     let (free_min, free_max, free_avg) = if free_count > 0 {
//         (free_min, free_max, free_sum / (free_count as u128))
//     } else { (0, 0, 0) };

//     klog!("[HEAP TIMING] Allocations (us): min={:.2}, max={:.2}, avg={:.2}\n",
//             alloc_min as f64 / 1000.0,
//             alloc_max as f64 / 1000.0,
//             alloc_avg as f64 / 1000.0);
//     klog!("  Histogram(%) <1us to <10us: ");
//     for i in 0..10 {
//         klog!("{:.2} | ", alloc_hist[i] as f64 / alloc_count as f64 * 100.0);
//     }
//     klog!("10us+: {:.2}%\n", alloc_hist[10] as f64 / alloc_count as f64 * 100.0);
//     klog!("[HEAP TIMING] Frees       (us): min={:.2}, max={:.2}, avg={:.2}\n",
//             free_min as f64 / 1000.0,
//             free_max as f64 / 1000.0,
//             free_avg as f64 / 1000.0);
//     klog!("  Histogram(%) <1us to <10us: ");
//     for i in 0..10 {
//         klog!("{:.2} | ", free_hist[i] as f64 / free_count as f64 * 100.0);
//     }
//     klog!("10us+: {:.2}%\n", free_hist[10] as f64 / free_count as f64 * 100.0);

// }

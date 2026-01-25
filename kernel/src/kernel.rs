//
// BlightOS Kernel
//
// Root Module
//
// 
#![no_std]
#![no_main]
#![feature(linked_list_retain)]

// Imports from the toolchain this is part of the toolchain
extern crate alloc; 

// Include all relevant kernel code here for everybody else to use
//// Standard utilities ////
#[macro_use]
pub mod util;
//// Architecture-dependent code ////
pub mod arch;
//// Various Memory Managers ////
pub mod mem;
//// Task Scheduler ////
pub mod sched;
//// Device Drivers ////
pub mod drivers;

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use alloc::boxed::Box;
use alloc::vec::Vec;
use util::*;
use crate::arch::*;
use crate::mem::phys::{pmm_num_free_frames};
use crate::mem::virt::AddressSpace;
use crate::sched::{SCHEDULER, Scheduler, Task, WaitChannel};


unsafe extern "C" {
    static _KERNEL_START: usize;
    static _KERNEL_END: usize;
}

static BSP_INITIALIZED : AtomicBool = AtomicBool::new(false);

#[global_allocator]
static ALLOCATOR: mem::heap::Kalloc = mem::heap::Kalloc::new();

static BSP_T1_TID: AtomicUsize = AtomicUsize::new(0);
static BSP_T2_TID: AtomicUsize = AtomicUsize::new(0);
static BSP_T3_TID: AtomicUsize = AtomicUsize::new(0);

pub struct RamdiskInfo {
    // Start/End of the physical address where the image is copied
    pub start_phy_addr: usize,
    pub end_phy_addr:   usize
}
static USER_INIT_PID: AtomicUsize = AtomicUsize::new(0);

// kstart : Kernel's Generic Entry Point
// This function will be called by all onlined CPUs (BSP with cpuid=0) in any
// order, albeit only after all onlined CPUs have reported to the arch-specific
// stub code so that the generic code has the correct CPU count (e.g., for 
// resource allocation purposes).
pub fn kstart(cpuid: usize,
                mmap_opt: Option<&[mem::phys::PMMapElement]>,
                ramdisk: Option<RamdiskInfo>)
{
    if cpuid == 0 {
        // BSP-only initialization
        klog!("BlightOS - Number of CPUs online: {}\n", cpu_count());
        let initramdisk : bool;

        match mmap_opt {
            Some(mmap) => {
                let kernel_start: usize;
                let kernel_end: usize;
                unsafe{
                    kernel_start = &_KERNEL_START as *const usize as usize;
                    kernel_end = &_KERNEL_END as *const usize as usize;
                }
                // Initialize the physical memory manager
                mem::phys::pmm_init(mmap, kernel_start, kernel_end);
            },
            _ => {panic!("No memory map was sent to the BSP!")}
        }
    
        // Mark the initramfs as used in the physical memory
        if let Some(initelf) = ramdisk {
            unsafe {
                let ep_virt = ((initelf.start_phy_addr + 0x18) as *const usize)
                                .read();
                klog!("InitELF: {:X} to {:X}\n", 
                        initelf.start_phy_addr,  initelf.end_phy_addr);
                // Frame aligned address range of the image
                let first_addr= round_down!(initelf.start_phy_addr,
                                            mem::phys::PHY_FRAME_SIZE);
                let last_addr = round_up!(initelf.end_phy_addr,
                                            mem::phys::PHY_FRAME_SIZE);
                let frame_cnt = (last_addr - first_addr) /
                                mem::phys::PHY_FRAME_SIZE;
                // Mark the frames as used
                mem::phys::pmm_mark_continuous(first_addr, frame_cnt, true);
                // Create the process address space
                match AddressSpace::spawn(first_addr, last_addr, ep_virt) {
                    Some(pid)   => {
                        USER_INIT_PID.store(pid, Ordering::Relaxed);
                        initramdisk = true;
                    },
                    None        => {
                        panic!("Could not create a process of INIT");
                    }
                }
            }
        } else {
            initramdisk = false;
        }

        // Load the drivers
        let drvs = drivers::get_builtin_drivers();
        for d in drvs.iter() {
            let ndevs = (d.enumerate)();
            klog!("Built-in Driver: {} - Enumerated {} device(s)\n",
                d.name, ndevs);
        }

        // Time keeping...
        SystemTimer::global_init(systimer_irq_handler);
        // Todo: Need an event timer to implement sleep, etc.

        // Install the system call handlers
        arch::syscall_register(SyscallOpCode::Exit,     syscall_exit);
        arch::syscall_register(SyscallOpCode::Open,     syscall_open);
        arch::syscall_register(SyscallOpCode::Read,     syscall_read);
        arch::syscall_register(SyscallOpCode::Write,    syscall_write);
        arch::syscall_register(SyscallOpCode::Exec,     syscall_exec);
        arch::syscall_register(SyscallOpCode::Close,    syscall_close);
        
        // End of serialized kernel startup. Let APs start!
        klog!("[KERNEL SELF-TEST]: Starting...\n");
        klog!("[TEST] Launching 2 tasks per CPU to compete over the same \
               counter and screen buffer\n");
        BSP_INITIALIZED.store(true, Ordering::Relaxed);

        // Create the inital task pool for this CPU
        Scheduler::config_round_robin(SysTimerDuration::Milliseconds(1));
        // Scheduler::config_first_come_first_served();
        BSP_T1_TID.store(Task::spawn(task1_exec), Ordering::Relaxed);
        BSP_T2_TID.store(Task::spawn(task2_exec), Ordering::Relaxed);

        // Spawn the first user-space task from the provided ramdisk if any
        if initramdisk == true {
            Task::spawn(user_init_exec);
        }
        // Jump to the first task and never come back ;)
        SCHEDULER.borrow_mut().start_scheduling(BSP_T1_TID.load(Ordering::Relaxed));
    } else {
        // AP initialization
        // Wait for BSP to perform the serialized portion of
        // kernel's initializaton
        while BSP_INITIALIZED.load(Ordering::Relaxed) == false {
            core::hint::spin_loop();
        }
        // Create the inital task pool for this CPU
        Scheduler::config_round_robin(SysTimerDuration::Milliseconds(1));
        Task::spawn(task2_exec);
        // Jump to the first task!
        SCHEDULER.borrow_mut().start_scheduling(Task::spawn(task1_exec));
    }
    panic!("Reached the end of kstart!");
}

fn systimer_irq_handler(_cpuid: u16){
    SCHEDULER.borrow_mut().preempt_irq();
}  

pub fn dump_memory(base: usize, qwords: usize) {
    unsafe {
        let mut datap: *mut usize = base as *mut usize;
        for _ in 0..qwords {
            klog!("{:X}: {:016X}\n", datap as usize, *datap);
            datap = datap.wrapping_add(1);
        }
    }
}

#[panic_handler]
pub fn panic(_info: &core::panic::PanicInfo) -> ! {
    arch::cpu_disable_ints();

    klog!("KERNEL BUG: {}\n", _info.message());

    let (fname, ln) = match  _info.location() {
        Some(loc) => (loc.file(), loc.line()),
        None => ("Unknown", 0)
    };
    // All paths start with "kernel/src/", so skip the first 11 chars
    if fname.len() > 11 {
        klog!("  LOCATION: {} @ ln {}\n", &fname[11..], ln);
    } else {
        klog!("  LOCATION: {} @ ln {}\n", fname, ln);
    }
    klog!("  CPU: {}\n", arch::cpu_id());
    arch::cpu_halt();
    loop {}
}

static SHARED_VAR : Spinlock<i32> = Spinlock::new(0);

// TESTING...
fn task1_exec() {
    let cpuid = *(THIS_CPU_ID.borrow());
    loop {
        {
            let mut shared_var = SHARED_VAR.lock();
            if *shared_var >= 100 {
                break;
            }
            *shared_var += 1;
            klog!("<C{}/T{}={}>", cpuid, Task::current(), *shared_var);
        }
        Task::sleep(Duration::from_millis(5));
    }
}

fn task2_exec() {
    let cpuid = *(THIS_CPU_ID.borrow());
    if cpuid == 0 {
        BSP_T3_TID.store(Task::spawn(task3_exec), Ordering::Relaxed); 
    }
    loop {
        {
            let mut shared_var = SHARED_VAR.lock();
            if *shared_var >= 100 {
                break;
            }
            *shared_var += 1;
            klog!("<C{}/T{}={}>", cpuid, Task::current(), *shared_var);
        }
        Task::sleep(Duration::from_millis(5));
    }
}

static WC : WaitChannel = WaitChannel::new();

fn task3_exec() {
    {
        SHARED_VAR.lock();
        klog!("<A LONG MESSAGE TO TEST THE CONSOLE PRINT_STR LOCK - TID{}>",
                Task::current());
    }
    // Wait for the first two tasks to finish
    Task::join(BSP_T1_TID.load(Ordering::Relaxed));
    Task::join(BSP_T2_TID.load(Ordering::Relaxed));
    Task::sleep(Duration::from_secs(1)); // Wait for task on other CPUs
    klog!("\n[Test] Parallel heap allocations - Free frames: {}\n",
            pmm_num_free_frames());

    let t4 = Task::spawn(|| {
        klog!("<Task {} allocate/verify/free 1000 i32>\n", Task::current());
        let mut myvec: Vec<i32> = Vec::new();
        for i in 0..1000 {
            myvec.push(i);
        }
        for i in 0..1000 {
            if myvec[i] != i as i32 {
                klog!("[FAIL] Vector element {} corrupted!\n", i);
                break;
            }
        }
    });
    
    {
        Task::sleep(Duration::from_millis(20));
        let _myvar1: Box<usize> = Box::new(1234);
        let _myvar2: Box<usize> = Box::new(2341);
        let _myvar3: Box<usize> = Box::new(3412);
        let _myvar4: Box<usize> = Box::new(4123);
        let mut myvec: Vec<i32> = Vec::new();
        klog!("<Task {} allocate/verify/free 1000 i32>\n", Task::current());
        for i in 0..1000 {
            myvec.push(i);
        }
        klog!("[MIDPOINT] Free frames: {}\n", pmm_num_free_frames());
        klog!("_myvars: {}, {}, {}, {}\n",
                    *_myvar1, *_myvar2, *_myvar3, *_myvar4);
        for i in 0..1000 {
            if myvec[i] != i as i32 {
                klog!("[FAIL] Vector element {} corrupted!\n", i);
                break;
            }
        }
    }
    Task::join(t4);
    klog!("Free frames: {}, Cached TLSF Metadata: 5\n", pmm_num_free_frames());
    klog!("[TEST] Co-op scheduling\n");
    let t5 = Task::spawn(|| {
        for _i in 0..10 {
            klog!("<T5>");
            Task::preempt();
        }
    });

    for _i in 0..10 {
        klog!("<T3>");
        Task::preempt();
    }

    Task::join(t5);
    klog!("\n[TEST] Shared wait channel...\n");
    let mut wtid : [usize; 5] = [0; 5];
    for i in 0..5 {
        wtid[i] = 
            Task::spawn(|| {
                klog!("<New task {} waiting on wc>", Task::current());
                WC.wait();
                klog!("<Task {} out of wait>", Task::current());
            });
    }
    Task::sleep(Duration::from_millis(1000));
    klog!("\n<Task {} Signaling all the waiters>\n", Task::current());
    WC.signal_all();
    for i in 0..5 {
        if wtid[i] > 0 {
            Task::join(wtid[i]);
        }
    }
    klog!("\n[KERNEL SELF-TEST] Finished - Free frames: {}\n",
        pmm_num_free_frames());
}

//////
/// The initial user-space task (if RAMDISK was provided by the bootloader)
///
fn user_init_exec() {
    // Wait for kernel's self-test to start and finish:
    while BSP_T3_TID.load(Ordering::Relaxed) == 0 {
        Task::preempt();
    }
    Task::join(BSP_T3_TID.load(Ordering::Relaxed));

    // Spawn the initial user-space process and convert this task into a
    // user-space task in the initial process!
    
    {
        SHARED_VAR.lock();
        klog!("[INIT] Task {} - Switching to the INIT user-space process..\n",
            Task::current());
    }
    AddressSpace::launch(USER_INIT_PID.load(Ordering::Relaxed));
}


///////
/// User-space interface
/// 

#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum SyscallOpCode {
    Exit            = 0,
    Open            = 1,
    Read            = 2,
    Write           = 3,
    Exec            = 4,
    Close           = 5,
    Max             = 6
}
pub enum Syscall {
    Exit{status: usize},
    Open{path_ptr: usize, mode: usize, ret_ptr: usize},
    Read{fd: usize, buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Write{fd: usize, buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Exec{fd: usize, cmd_buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Close{fd: usize},
}
pub type SyscallHandlerFn = fn(usize, usize, usize, usize);

fn syscall_exit(status: usize, _: usize, _: usize, _: usize) {
    {
        SHARED_VAR.lock();
        klog!("\nProgram Exited with status {} - Current TID:{}\n",
            status, Task::current());
    }
    Task::exit();
}

fn syscall_open(path_ptr: usize, mode: usize, _ret_ptr: usize, _: usize) {
    SHARED_VAR.lock();
    klog!("\nSyscall: OPEN({}, {}) by TID:{}\n", 
                path_ptr, mode, Task::current());
}

fn syscall_read(fd: usize, buf: usize, len: usize, _ret_ptr: usize) {
    if fd == 0 && len >= 1 {
        // Standard input - i8046 keyboard
        unsafe {
            // return the last keychar to the user-space
            let out = core::slice::from_raw_parts_mut(buf as *mut u8, len);
            out[0] = drivers::I8046::read_key_ascii();
                
        }
    }
}

fn syscall_write(fd: usize, buf: usize, len: usize, _ret_ptr: usize) {
    SHARED_VAR.lock();
    if fd == 0 {
        // Standard output - VGA console
        unsafe {
            kearly_console::print_str(
                        core::slice::from_raw_parts(buf as *const u8, len));
            
        }
    }
    
}

fn syscall_exec(fd: usize, _cmd_ptr: usize, _cmd_len: usize, _ret_ptr: usize) {
    SHARED_VAR.lock();
    klog!("\nSyscall: EXEC({}, {}) by TID:{}\n", fd, _cmd_ptr, Task::current());
}

fn syscall_close(fd: usize, _: usize, _: usize, _: usize) {
    SHARED_VAR.lock();
    klog!("\nSyscall: CLOSE({}) by TID:{}\n", fd, Task::current());
}

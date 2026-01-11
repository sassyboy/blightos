//
// BlightOS Kernel
//
// Root Module
//
// 
#![no_std]
#![no_main]

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

use core::sync::atomic::{AtomicBool, Ordering};
use alloc::boxed::Box;
use alloc::vec::Vec;
use util::*;
use crate::arch::*;
use crate::mem::physical::pmm_num_free_frames;
use crate::sched::SCHEDULER;


unsafe extern "C" {
    static _KERNEL_START: usize;
    static _KERNEL_END: usize;
}

static BSP_INITIALIZED : AtomicBool = AtomicBool::new(false);

#[global_allocator]
static ALLOCATOR: mem::heap::Kalloc = mem::heap::Kalloc::new();

// kstart : Kernel's Generic Entry Point
// This function will be called by all onlined CPUs (BSP with cpuid=0) in any
// order, albeit only after all onlined CPUs have reported to the arch-specific
// stub code so that the generic code has the correct CPU count (e.g., for 
// resource allocation purposes).
pub fn kstart(cpuid: usize, mmap_opt: Option<&[mem::physical::PMMapElement]>) {
    if cpuid == 0 {
        // BSP-only initialization
        klog!("BlightOS - Number of CPUs online: {}\n", cpu_count());

        match mmap_opt {
            Some(mmap) => {
                let kernel_start: usize;
                let kernel_end: usize;
                unsafe{
                    kernel_start = &_KERNEL_START as *const usize as usize;
                    kernel_end = &_KERNEL_END as *const usize as usize;
                }
                // Initialize the physical memory manager
                mem::physical::pmm_init(mmap, kernel_start, kernel_end);
            },
            _ => {panic!("No memory map was sent to the BSP!")}
        }
    
        // Time keeping...
        SystemTimer::global_init(systimer_irq_handler);
        // Todo: Need an event timer to implement sleep, etc.

        // End of serialized kernel startup. Let APs start!
        klog!("Launching 2 tasks per CPU to compete over the same counter and \
                screen buffer...\n");
        BSP_INITIALIZED.store(true, Ordering::Relaxed);

        // Create the inital task pool for this CPU
        let sched = SCHEDULER.borrow_mut();
        sched.config_round_robin(SysTimerDuration::Milliseconds(1));
        sched.create_task(0, idle_task, 1);
        sched.create_task(1, task1_exec, 2);
        sched.create_task(2, task2_exec, 4);
        // Jump to the first task and never come back ;)
        sched.start_scheduling(1, 0);
    } else {
        // AP initialization

        // Wait for BSP to perform the serialized portion of
        // kernel's initializaton
        while BSP_INITIALIZED.load(Ordering::Relaxed) == false {
            core::hint::spin_loop();
        }
        // Create the inital task pool for this CPU
        let sched = SCHEDULER.borrow_mut();
        sched.config_round_robin(SysTimerDuration::Milliseconds(1));
        sched.create_task(0, idle_task, 1);
        sched.create_task(1, task1_exec, 2);
        sched.create_task(2, task2_exec, 3);
        // Jump to the first task!
        sched.start_scheduling(1, 0);
    }
    panic!("Reached the end of kstart!");
}

fn systimer_irq_handler(_cpuid: u16){
    SCHEDULER.borrow().preempt();
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
            klog!("<C{}/T1:{}>", cpuid, *shared_var);
        }
        arch::cpu_busywait_us(10_000);
    }
}

fn task2_exec() {
    let cpuid = *(THIS_CPU_ID.borrow());
    if cpuid == 0 {
        SCHEDULER.borrow_mut().create_task(3, task3_exec, 1);
    }
    loop {
        {
            let mut shared_var = SHARED_VAR.lock();
            if *shared_var >= 100 {
                break;
            }
            *shared_var += 1;
            klog!("<C{}/T2:{}>", cpuid, *shared_var);
        }
        arch::cpu_busywait_us(20_000);
    }
}

fn task3_exec() {
    klog!("<THIS IS A LONG MESSAGE TO TEST THE CONSOLE PRINT_STR LOCK>");
    cpu_busywait_us(1_000_000 * 3); // wait 3 seconds

    klog!("\n[START] Heap Allocator Test - FREE FRAMES: {}\n",
            pmm_num_free_frames());
    {
        let _myvar1: Box<usize> = Box::new(1234);
        let _myvar2: Box<usize> = Box::new(2341);
        let _myvar3: Box<usize> = Box::new(3412);
        let _myvar4: Box<usize> = Box::new(4123);
        let mut myvec: Vec<i32> = Vec::new();
        for i in 0..1000 {
            myvec.push(i);
        }
        klog!("[MIDPOINT] FREE FRAMES: {}\n", pmm_num_free_frames());
        klog!("_myvars: {}, {}, {}, {}\n",
                    *_myvar1, *_myvar2, *_myvar3, *_myvar4);
        for i in 0..1000 {
            if myvec[i] != i as i32 {
                klog!("[FAIL] Vector element {} corrupted!\n", i);
                break;
            }
        }
    }
    klog!("[FINISHED] Heap Allocator Test - FREE FRAMES: {}\n",
            pmm_num_free_frames());    
}

fn idle_task() {
    // Halt puts the CPU in low-power mode and stops exection until there's an
    // interrupt. Each CPU should end up here if there is no task to run
    let cpuid = *(THIS_CPU_ID.borrow());
    {
        SHARED_VAR.lock();
        klog!("<CPU{}/IDLE>", cpuid);
        if cpuid == 0 {
            klog!("FINAL FREE FRAMES: {}\n", pmm_num_free_frames());
        }
    }
    loop {
        arch::cpu_enable_ints();
        arch::cpu_halt();
    }
}
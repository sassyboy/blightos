//
// BlightOS Kernel
//
// Root Module
//
// 
#![no_std]
#![no_main]

// Include all relevant kernel code here for everybody else to use
// Architecture-dependent code
pub mod arch;
// Standard utilities
#[macro_use]
pub mod util;


use core::sync::atomic::{AtomicBool, Ordering};
use util::*;
use crate::arch::*;
use crate::sched::SCHEDULER;

// Physical Memory Manager
#[path = "mem/pmm.rs"]
pub mod pmm;
// Task Scheduler
pub mod sched;


unsafe extern "C" {
    static _KERNEL_START: usize;
    static _KERNEL_END: usize;
}

static BSP_INITIALIZED : AtomicBool = AtomicBool::new(false);

// kstart : Kernel's Generic Entry Point
// This function will be called by all onlined CPUs (BSP with cpuid=0) in any
// order, albeit only after all onlined CPUs have reported to the arch-specific
// stub code so that the generic code has the correct CPU count (e.g., for 
// resource allocation purposes).
pub fn kstart(cpuid: usize, mmap_opt: Option<&[pmm::PMMapElement]>) {
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
                pmm::pmm_init(mmap, kernel_start, kernel_end);
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
}

fn idle_task() {
    // Halt puts the CPU in low-power mode and stops exection until there's an
    // interrupt. Each CPU should end up here if there is no task to run
    let cpuid = *(THIS_CPU_ID.borrow());
    {
        SHARED_VAR.lock();
        klog!("<CPU{}/IDLE>", cpuid);
    }
    loop {
        arch::cpu_enable_ints();
        arch::cpu_halt();
    }
}
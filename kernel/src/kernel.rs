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
use util::*;
// Physical Memory Manager
#[path = "mem/pmm.rs"]
pub mod pmm;
// Task Scheduler
pub mod sched;


unsafe extern "C" {
    static _KERNEL_START: usize;
    static _KERNEL_END: usize;
}

pub fn kstart(mmap: &[pmm::PMMapElement]) {

    // Print the E820 memory map
    klog!("BlightOS - A practice OS for me to learn Rust!\n");
    klog!("Physical Memory Map:\n");
    for item in mmap {
        klog!("{:016X} - {:016X}: {}\n",
            item.base, item.base + item.len - 1,
            match item.avail {
                true => "[USABLE]",
                false=> "[RESERV]"
            }
        );
    }

    // Print the kernel image range
    let kernel_start: usize;
    let kernel_end: usize;
    unsafe{
        kernel_start = &_KERNEL_START as *const usize as usize;
        kernel_end = &_KERNEL_END as *const usize as usize;
    }
    klog!("Kernel Image [{:016X} - {:016X}], {:.2} MB\n",
        kernel_start, kernel_end, 
        ((kernel_end - kernel_start) as f64)/(1024 * 1024) as f64
    );

    // Initialize the physical memory manager
    pmm::pmm_init(); // Todo: simple bitmap of PAGE_SIZE should do

    // Enable the interrupts and the system timer
    arch::systimer_set_periodic(1000, ktick);
    arch::irq_controller_init();
    arch::cpu_enable_ints();

    // Todo Initialize the scheduler and spawn the init process
    sched::new_task(0, idle_task);
    sched::new_task(1, task1_exec);
    sched::new_task(2, task2_exec);
    sched::start_scheduling(1);    
    panic!("Reached the end of kstart!");
}

fn ktick(_: u16){
    // klog!("!");
    sched::preempt();
}

pub fn dump_memory(base: usize, qwords: usize) {
    unsafe {
        let mut datap: *mut usize = base as *mut usize;
        for _ in 0..qwords {
            klog!("{:016X}\n", *datap);
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


// TESTING...
fn task1_exec() {
    for _ in 0..20 {
        if let Ok(_lock) = sched::Preemption::lock() {
            // Preemption-free section
            klog!("<T1>");
            
        }
        arch::cpu_busywait(10_000_000);
    }
}

fn task2_exec() {
    sched::new_task(3, task3_exec);
    for _ in 0..40 {
        if let Ok(_lock) = sched::Preemption::lock() {
            // Preemption-free section
            klog!("<T2>");
        }
        arch::cpu_busywait(10_000_000);
    }
}

fn task3_exec() {
    if let Ok(_lock) = sched::Preemption::lock() {
        // Preemption-free section
        klog!("<THIS IS A LONG MESSAGE TO TEST THE PREEMPTION LOCK>");
    } 
}

fn idle_task() {
    // Halt puts the CPU in low-power mode and stops exection until there's an
    // interrupt. Each CPU should end up here if there is no task to run
    klog!("<IDLE>");
    loop {
        arch::cpu_enable_ints();
        arch::cpu_halt();
    }
}
//
// BlightOS Kernel
//
// Architecture Dependent Implementation
//   Selects the right low-level stub code for the rest of the kernel
// 

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("Unsupported architecture!");

#[cfg(target_arch = "x86_64")]
#[path = "x86_64/stub.rs"]
mod asc;

#[cfg(target_arch = "aarch64")]
#[path = "aarch64/stub.rs"]
mod asc;

// Re-export the architecture-specific code at the top of this module
pub use self::asc::*;

//
// Common interfaces implemented by various architecture implementation module
//

//
// Memory Management Unit Interface
//
use crate::mem::MemoryType;
pub trait MMUTrait {

    //
    // MMU Initialization Methods
    //

    /// Initializes the MMU subsystem for the entire system.
    /// Initializes the dynamic mapping area (kmap), which is a reserved virtual
    /// address range that the kernel (and its clients, such as device drivers)
    /// can use for mapping/unmapping physical memory for their operations
    /// (e.g., DMA) that is shared amont all process address-spaces.
    /// 
    /// This is called by the architecture-specific BSP initialization code
    /// before any kernel modules are initialized.
    fn global_init();

    /// Initializes the MMU subsystem for the current CPU.
    /// This is called by the architecture-specific per-CPU initialization code
    /// on each CPU after the global initialization is done, and before any
    /// kernel modules are initialized.
    fn per_cpu_init();

    //
    // Kernel Dynamic Mapping Methods
    //

    /// Finds a virtual address range in the kmap pool, and maps the physical
    /// frames specified by `phys_base` and `frame_cnt` to that virtual address
    /// range with the appropriate flags for the given `cache` type
    /// 
    /// If successful, it returns the base virtual address of the mapped range.
    /// The resulting mapping is continuous both in virtual and physical memory.
    fn kmap(phys_base: usize, frame_cnt: usize, cache: MemoryType)
                                                            -> Option<usize>;

    /// Unmaps the virtual address range starting at `virt_base` and covering
    /// `frame_cnt` frames in the kmap area, and makes it available for future
    /// mapping requests. The caller is responsible for ensuring that the
    /// given virtual address range is valid and currently mapped
    fn kunmap(virt_base: usize, frame_cnt: usize);

    //
    // User-Space Mapping Methods
    //

    /// Initializes the MMU data structures representing a single process's
    /// virtual address space.
    /// The mapping must include kernel's address space to begin with.
    fn init(&mut self);

    /// Maps a number of pages (virtual address starting from `virt_addr`) to
    /// frames (physical address) for a user-space process.
    /// 
    /// The caller must have already reserved a number of frames from the
    /// physical memory manager and left their addresses in `phys_addrs`.
    /// The frames don't not have to be physically contiguous, but resulting
    /// virtual address range will be contiguous. The caller is responsible for
    /// ensuring that the given virtual address range is valid and currently
    /// unmapped. If the virtual address is already mapped, the function
    /// returns false without rolling back any partial changes to the
    /// address space.
    fn map_pages(&mut self, virt_addr: usize, phys_addrs: &[usize],
                     writeable: bool, exec: bool, cache: MemoryType) -> bool;
    
    /// Unmaps the page starting at `virt_addr` and makes it available for
    /// future mapping requests. The caller is expected to free the physical
    /// frame after unmapping.
    /// Returns the physical address of the unmapped page if successful, or
    /// None
    fn unmap_page(&mut self, virt_addr: usize) -> Option<usize>;

    /// Unmaps the given virtual address range and returns the number of pages
    /// that were unmapped.
    /// The caller is expected to know the physical addresses of the unmapped
    /// pages and free them after unmapping as this function does not return the
    /// physical addresses.
    fn unmap_pages(&mut self, virt_addr: usize, num_pages: usize) -> usize;

    //
    // Address-space Switching Methods
    //

    /// Switches to the address space represented by this MMUMapping object.
    fn enter(&self);

    /// Converts the currently running kernel task into a user-space task as a
    /// part of this process address space. The calling (kernel) task will not
    /// return to the next instruction after its call to move_to_userspace.
    /// The user-space execution must end with an Exit system call, at which
    /// point the task terminates.
    fn move_to_userspace(priv_data: usize, entry_point: usize, arg: usize,
                            user_stack: usize, exit_handler: fn()) -> !;

    /// Returns a copy of the architecture-specific data of the currently
    /// running address-space, which will be used in address-space management,
    /// e.g., to call `move_to_userspace` when launching a new user-space task.
    fn copy_priv_data(&self) -> usize;

    //
    // Misc Methods
    //

    /// Flushes the TLBs of all cores.
    fn flush_tlbs();

    /// Flushes the TLB entry for the given virtual address on all cores.
    fn flush_tlb_for_page(virt_addr: usize);

    /// Finds the physical address that is mapped to the given virtual address
    /// by walking the paging structures of the given address space.
    /// Returns None if the virtual address is not mapped.
    fn virt_to_phys(&self, virt_addr: usize) -> Option<usize>;
}
//
// IRQ/SYSCALL Interface
//
type IsrHandlerFn = fn(u16);

//
// Kernel Timer Interace
//
use core::time::Duration;
pub enum SysTimerMode {
    OneShot,
    Periodic,
    Disabled
}

pub trait SystemTimerTrait {
    // To be called once during kernel's serialized initialization to install a
    // single IRQ handler. Every core will execute the same handler code, even
    // though each having an individual timer (and set of events)
    fn global_init(isr_callback: IsrHandlerFn);

    /// Called on every core during the initialization of the CPU in kernel.rs
    /// At this point global_init has already been called.
    fn per_cpu_init();

    fn exec_handler();

    // Per-CPU
    fn set_mode(mode: SysTimerMode);

    // Per-CPU
    // Sets the period of IRQs or the timestamp of the next IRQ to generate
    // depending on the mode set for the timer.
    fn arm(duration: Duration);

    fn frequency_hz() -> u64;

    fn duration_to_timestamp_ticks(d: Duration) -> u64;

    fn timestamp_to_duration(t: u64) -> Duration;

    fn current_timestamp() -> u64;

    fn current_timestamp_as_duration() -> Duration {
        Self::timestamp_to_duration(Self::current_timestamp())
    }
}

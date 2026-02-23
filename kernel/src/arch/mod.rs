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
// MMU
//
pub enum MmuCachingPolicy {
    NonCaching,    // Slow Memory RW - Totally safe for MMIO/DMA
    WriteThrough,  // Fast Memory R, Slow Memory/MMIO W - Not safe for MMIO Read
    WriteBack      // Fast Memory-only R/W - Not for MMIO
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

    fn exec_handler();

    // Per-CPU
    fn set_mode(mode: SysTimerMode);

    // Per-CPU
    // Sets the period of IRQs or the timestamp of the next IRQ to generate
    // depending on the mode set for the timer.
    fn arm(duration: Duration);

    fn duration_to_timestamp_ticks(d: Duration) -> u64;

    fn timestamp_to_duration(t: u64) -> Duration;

    fn current_timestamp() -> u64;

    fn current_timestamp_as_duration() -> Duration {
        Self::timestamp_to_duration(Self::current_timestamp())
    }
}

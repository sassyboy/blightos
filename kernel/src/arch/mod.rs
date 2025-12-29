//
// BlightOS Kernel
//
// Architecture Dependent Implementation
//   Selects the right low-level stub code for the rest of the kernel
// 

#[cfg(not(any(target_arch = "x86_64", target_arch = "arm")))]
compile_error!("Unsupported architecture!");

#[cfg(target_arch = "x86_64")]
#[path = "x86_64/stub.rs"]
mod asc;

// #[cfg(target_arch = "x86_64")]
// #[path = "arch/x86_64/stub.rs"]
// mod internal;

// Re-export the architecture-specific code at the top of this module
pub use self::asc::*;


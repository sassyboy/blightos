//
// BlightOS Kernel
//
// Device Driver Interface
//
//

use core::sync::atomic::Ordering::Relaxed;
use alloc::{vec, vec::*};
use crate::drivers::machine::Machine;
use crate::{arch, drivers::pci::PCIBus};
use crate::drivers::storage::ahci::AHCIBus;
use crate::drivers::kbd::I8046Keyboard;
use crate::util::*;

pub mod machine;
pub mod pci;
pub mod kbd;
pub mod storage;


pub struct DriverInfo {
    pub name:       &'static str,
    // Detect and allocate resources for comaptible device and return the number
    // of devices. Called by CPU0 without IRQs enabled
    pub enumerate:  fn() -> usize,

    // Called after multiprocessing and scheduling is enabled in the kernel by
    // the init task.
    // Things like spawing tasks (workers) should be done in this phase
    pub post_enum:  fn(),

    // Release the device (and corresponding resources) corresponding to the
    // specified index.
    pub release:    fn(usize),

}

fn noop() {}

pub fn get_builtin_drivers() -> Vec<DriverInfo> {
    vec![
        DriverInfo {
            name: "Machine",
            enumerate:  Machine::enumerate,
            post_enum:  Machine::post_enum,
            release:    Machine::release
        },
        DriverInfo {
            name: "PCI Bus",
            enumerate: PCIBus::enumerate,
            post_enum: noop,
            release: PCIBus::release
        },
        DriverInfo {
            name: "i8046 PS/2 Controller",
            enumerate: I8046Keyboard::enumerate,
            post_enum: noop,
            release: I8046Keyboard::release
        },
        DriverInfo {
            name: "AHCI/SATA Bus",
            enumerate: AHCIBus::enumerate,
            post_enum: AHCIBus::post_enum,
            release: AHCIBus::release
        },

        #[cfg(target_arch = "arm")]
        DriverInfo {
            name: "ARM-UART",
            enumerate: I8046::enumerate,
            release: I8046::release
        }
    ]
}


//
// Structures that have to be encodeded/decoded to/from a packed/specific format
// in the memory for/by devices controllers implement this trait.
// Similar to #[repr(C, packed)] structures with .read/write_volatile, but with
// more freedom in how the structure fields are defined and named
//
pub trait DeviceStruct {
    fn encode(&self, dest_addr: usize);
    fn decode(&mut self, src_addr: usize);
}

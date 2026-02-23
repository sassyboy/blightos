//
// BlightOS Kernel
//
// Device Driver Interface
//
//

pub mod machine;
#[cfg(target_arch = "x86_64")]
pub mod pci;
pub mod input;
pub mod storage;
pub mod video;

use alloc::{vec, vec::*};
use crate::drivers::machine::Machine;
use crate::drivers::video::framebuffer::FrameBuffer;

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
        // Common Drivers
        DriverInfo {
            name: "Machine",
            enumerate:  Machine::enumerate,
            post_enum:  Machine::post_enum,
            release:    Machine::release
        },
        DriverInfo {
            name: "FrameBuffer",
            enumerate:  FrameBuffer::enumerate,
            post_enum:  FrameBuffer::post_enum,
            release:    FrameBuffer::release
        },
        // X64-64 only support
        #[cfg(target_arch = "x86_64")]
        DriverInfo {
            name: "PCI Bus",
            enumerate: crate::drivers::pci::PCIBus::enumerate,
            post_enum: noop,
            release: crate::drivers::pci::PCIBus::release
        },
        #[cfg(target_arch = "x86_64")]
        DriverInfo {
            name: "i8046 PS/2 Controller",
            enumerate: crate::drivers::input::i8046::I8046Keyboard::enumerate,
            post_enum: noop,
            release: crate::drivers::input::i8046::I8046Keyboard::release
        },
        #[cfg(target_arch = "x86_64")]
        DriverInfo {
            name: "AHCI/SATA Bus",
            enumerate: crate::drivers::storage::ahci::AHCIBus::enumerate,
            post_enum: crate::drivers::storage::ahci::AHCIBus::post_enum,
            release: crate::drivers::storage::ahci::AHCIBus::release
        },
        // AARCH64 only support
        #[cfg(target_arch = "aarch64")]
        DriverInfo {
            name: "UART Keyboard",
            enumerate: crate::drivers::input::uartkbd::UARTKeyboard::enumerate,
            post_enum: crate::drivers::input::uartkbd::UARTKeyboard::post_enum,
            release: crate::drivers::input::uartkbd::UARTKeyboard::release
        },
        #[cfg(target_arch = "aarch64")]
        DriverInfo {
            name: "eMMC Controller",
            enumerate: crate::drivers::storage::emmc::BCM2835SDHost::enumerate,
            post_enum: crate::drivers::storage::emmc::BCM2835SDHost::post_enum,
            release: crate::drivers::storage::emmc::BCM2835SDHost::release
        },
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

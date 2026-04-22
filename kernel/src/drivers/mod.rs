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
pub mod audio;
pub mod video;
pub mod gui;

use alloc::{vec, vec::*};
use crate::arch::{MMUMapping, MMUTrait};
use crate::mem::MemoryType;
use crate::drivers::machine::Machine;
use crate::drivers::video::framebuffer::FrameBuffer;
use crate::drivers::gui::GUI;
use crate::mem::phys::PhysMem;

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
            name:       "Machine",
            enumerate:  Machine::enumerate,
            post_enum:  Machine::post_enum,
            release:    Machine::release
        },
        DriverInfo {
            name:       "FrameBuffer",
            enumerate:  FrameBuffer::enumerate,
            post_enum:  FrameBuffer::post_enum,
            release:    FrameBuffer::release
        },
        DriverInfo {
            name:       "GUI Server",
            enumerate:  GUI::enumerate,
            post_enum:  GUI::post_enum,
            release:    GUI::release
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
            enumerate: crate::drivers::input::i8042::I8042::enumerate,
            post_enum: noop,
            release: crate::drivers::input::i8042::I8042::release
        },
        #[cfg(target_arch = "x86_64")]
        DriverInfo {
            name: "AHCI/SATA Bus",
            enumerate: crate::drivers::storage::ahci::AHCIBus::enumerate,
            post_enum: crate::drivers::storage::ahci::AHCIBus::post_enum,
            release: crate::drivers::storage::ahci::AHCIBus::release
        },
        #[cfg(target_arch = "x86_64")]
        DriverInfo {
            name: "Intel HDA Audio Controller",
            enumerate: crate::drivers::audio::intel_hda::IntelHDA::enumerate,
            post_enum: crate::drivers::audio::intel_hda::IntelHDA::post_enum,
            release: crate::drivers::audio::intel_hda::IntelHDA::release
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


/// 
/// MMIOAccessible is a marker trait for types that can be read/written as
/// memory-mapped I/O (MMIO) registers.
/// Device drivers can implement MMIOAccessible for their custom types/structs
/// if needed, but basic integer types are already implemented for convenience.
pub trait MMIOAccessible{}
impl MMIOAccessible for u8 {}
impl MMIOAccessible for i8 {}
impl MMIOAccessible for u16 {}
impl MMIOAccessible for i16 {}
impl MMIOAccessible for u32 {}
impl MMIOAccessible for i32 {}
impl MMIOAccessible for u64 {}
impl MMIOAccessible for i64 {}

///
/// Encapsulates a contiguous region of physical memory assigned to a device
/// memory-mapped I/O (MMIO).
/// 
/// This struct handles two types of register files:
/// - Logical: A logical register file is a sub-region of a larger MMIO space
///   that has already been mapped to kernel's virtual address space.
/// - Physical: A physical register file is a MMIO region that needs to be
///   mapped to the kernel's virtual address space by this struct.
///   Upon initialization, the struct marks the physical memory as used, maps it
///   into the kernel's virtual address space as MemoryType::Device,
///   and provides the virtual address for device drivers to use.
///   Upon dropping, the struct unmaps the virtual address but doesn't mark the
///   physical memory as free.
#[derive(Debug, Clone)]
pub struct RegisterFile {
    pub base_phys: usize,
    pub base_virt: usize,
    pub length: usize,  // Size of the MMIO region in bytes
    frames: usize,      // Number of pages/frames mapped
    logical: bool,      // i.e., already mapped
}
impl RegisterFile {
    pub const fn new() -> Self {
        Self {
            base_phys:  0,
            base_virt:  0,
            length:     0,
            frames:     0,
            logical:    false,
         }
    }
    pub fn init_physical(&mut self, phys: usize, length: usize) {
        self.logical = false;
        self.base_phys = phys;
        self.length = length;
        self.frames = div_round_up!(length, MMUMapping::PAGE_SIZE);
        self.base_virt = MMUMapping::kmap(phys, self.frames, MemoryType::Device)
                                    .expect("Failed to map device MMIO region");
        PhysMem::mark_continuous(phys, self.frames, true);
    }

    pub fn init_logical(&mut self, virt: usize, length: usize) {
        self.logical = true;
        self.base_virt = virt;
        self.length = length;
    }

    pub fn write<T: MMIOAccessible>(&mut self, offset: usize, value: T) {
        if offset + core::mem::size_of::<T>() > self.length {
            panic!("Out-of-bound MMIO write: {} bytes @ offset {}, \
                    mmio_base: {:#X}, mmio_size: {}",
                    core::mem::size_of::<T>(), offset,
                    self.base_virt, self.length);
        }
        let addr = (self.base_virt + offset) as *mut T;
        unsafe { addr.write_volatile(value) }
    }

    pub fn read<T: MMIOAccessible>(&self, offset: usize) -> T {
        if offset + core::mem::size_of::<T>() > self.length {
            panic!("Out-of-bound MMIO read: {} bytes @ offset {}, \
                    mmio_base: {:#X}, mmio_size: {}",
                    core::mem::size_of::<T>(), offset,
                    self.base_virt, self.length);
        }
        let addr = (self.base_virt + offset) as *const T;
        unsafe { addr.read_volatile() }
    }

    pub unsafe fn as_mut_ptr<T: MMIOAccessible>(&self, offset: usize) -> *mut T {
        if offset + core::mem::size_of::<T>() > self.length {
            panic!("Out-of-bound MMIO pointer access: {} bytes @ offset {}, \
                    mmio_base: {:#X}, mmio_size: {}",
                    core::mem::size_of::<T>(), offset,
                    self.base_virt, self.length);
        }
        (self.base_virt + offset) as *mut T
    }
}
impl Drop for RegisterFile {
    fn drop(&mut self) {
        if !self.logical && self.base_virt != 0 && self.frames > 0 {
            MMUMapping::kunmap(self.base_virt, self.frames);
            self.base_virt = 0;
            self.frames = 0;
        }
    }
}

///
/// Encapsulates a contiguous region of physical memory assigned to a device
/// to perform Direct Memory Access (DMA) operations.
/// 
/// Upon initialization, the struct marks the physical memory as used, maps it
/// into the kernel's virtual address space as MemoryType::Device/OutputDMA,
/// and provides the virtual address for device drivers to use.
/// Upon dropping, the struct unmaps the virtual address, and free the physical
/// memory if it was allocated by this struct (i.e., if allocated is true).
#[derive(Debug, Clone)]
pub struct DMABuffer {
    pub phys_addr: usize,
    pub virt_addr: usize,
    pub length: usize,
    frames: usize,
    preallocated: bool,
}
impl DMABuffer {
    pub const fn new() -> Self {
        Self {
            phys_addr:  0,
            virt_addr:  0,
            length:     0,
            frames:     0,
            preallocated:   false,
         }
    }

    ///
    /// Initialize the DMABuffer with a pre-allocated physical memory region.
    /// The caller is responsible for ensuring the physical memory region is
    /// valid and appropriately sized for the intended DMA operations.
    pub fn init_preallocated(&mut self, phys: usize, length: usize,
                                                            output_dma: bool) {
        let mem_type = if output_dma {
            MemoryType::OutputDMA
        } else {
            MemoryType::Device
        };
        self.preallocated = true;
        self.phys_addr = phys;
        self.length = length;
        self.frames = div_round_up!(length, MMUMapping::PAGE_SIZE);
        self.virt_addr = MMUMapping::kmap(phys, self.frames, mem_type)
                                    .expect("Failed to map DMA buffer");
        PhysMem::mark_continuous(phys, self.frames, true);
    }

    ///
    /// Initialize the DMABuffer by allocating a new contiguous physical memory
    /// region of at leats the specified length, and mapping it into the
    /// kernel's virtual address space.
    pub fn init(&mut self, length: usize, output_dma: bool) {
        let mem_type = if output_dma {
            MemoryType::OutputDMA
        } else {
            MemoryType::Device
        };
        self.preallocated = false;
        self.length = length;
        self.frames = div_round_up!(length, MMUMapping::PAGE_SIZE);
        self.phys_addr = PhysMem::alloc_continuous(self.frames)
                                        .expect("Out of memory");
        self.virt_addr = MMUMapping::kmap(self.phys_addr, self.frames, mem_type)
                                    .expect("Failed to map DMA buffer");
    }
}
impl Drop for DMABuffer {
    fn drop(&mut self) {
        if self.virt_addr != 0 && self.frames > 0 {
            MMUMapping::kunmap(self.virt_addr, self.frames);
            self.virt_addr = 0;
            if !self.preallocated {
                PhysMem::free_continuous(self.phys_addr, self.frames);
                self.phys_addr = 0;
            }
            self.frames = 0;
        }
    }
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

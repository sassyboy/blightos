//
// BlightOS Kernel
//
// PCI Bus Driver
//
#![allow(dead_code)]
use core::fmt;
use core::fmt::Display;
use alloc::vec::Vec;

#[cfg(feature="debug_pci")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[PCI] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_pci"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}

use crate::{arch::{x86_ioport_read, x86_ioport_write}, util::Spinlock};
#[derive(Clone, Copy)]
pub struct PCIDevice {
    pub bus_id:     u8,
    pub slot_id:    u8,
    pub func_id:    u8,
    pub vendor_id:  u16,
    pub device_id:  u16,
    pub class:      u8,
    pub sub_class:  u8,
    pub prog_if:    u8,
    pub revision_id:u8,
    pub bar:        [u32; 6],
    pub irq_line:   u8,
    pub irq_pin:    u8,
    pub valid:      bool,
}
impl PCIDevice {
    pub const fn new() -> Self {
        Self {
            bus_id:     0,
            slot_id:    0,
            func_id:    0,
            vendor_id:  0,
            device_id:  0,
            class:      0,
            sub_class:  0,
            prog_if:    0,
            revision_id:0,
            bar:        [0; 6],
            irq_line:   0,
            irq_pin:    0,
            valid:      false
        }
    }

    pub fn load_from_pci(&mut self, bus: u8, slot: u8, func: u8) {
        self.bus_id = bus;
        self.slot_id= slot;
        self.func_id= func;
        let reg0 = PCIBus::pci_read(bus, slot, func, 0);
        let reg2 = PCIBus::pci_read(bus, slot, func, 2 * 4);
        let regf = PCIBus::pci_read(bus, slot, func, 15 * 4);
        self.vendor_id  = (reg0 & 0xFFFF) as u16;
        self.device_id  = (reg0 >> 16) as u16;
        self.class      = ((reg2 >> 24) & 0xFF) as u8;
        self.sub_class  = ((reg2 >> 16) & 0xFF) as u8;
        self.prog_if    = ((reg2 >> 8 ) & 0xFF) as u8;
        self.revision_id= (reg2 & 0xFF) as u8;
        for i in 0..6 {
            self.bar[i] = PCIBus::pci_read(bus, slot, func,
                                    PCIBus::REG_BAR0 + i as u8 * 4);
        }
        self.irq_line   = (regf & 0xFF) as u8;
        self.irq_pin    = ((regf >> 8) & 0xFF) as u8;
        self.valid = true;
    }

    pub fn enable_memspace(&self) -> bool {
         if !self.valid {
            return false;
        }
        let mut cmdsts = PCIBus::pci_read(self.bus_id, self.slot_id,
                                        self.func_id, PCIBus::REG_COMMAND);
        cmdsts |= 0x2; // MEM Space Enable
        PCIBus::pci_write(self.bus_id, self.slot_id, self.func_id,
                            PCIBus::REG_COMMAND, cmdsts);
        true
    }

    pub fn disable_memspace(&self) -> bool {
        if !self.valid {
            return false;
        }
        let mut cmdsts = PCIBus::pci_read(self.bus_id, self.slot_id,
                                        self.func_id, PCIBus::REG_COMMAND);
        cmdsts &= !0x2; // MEM Space Disable
        PCIBus::pci_write(self.bus_id, self.slot_id, self.func_id,
                            PCIBus::REG_COMMAND, cmdsts);
        true
    }

    pub fn enable_bus_master(&self) -> bool {
        if !self.valid {
            return false;
        }
        let mut cmd = PCIBus::pci_read(self.bus_id, self.slot_id, self.func_id,
                                        PCIBus::REG_COMMAND);
        cmd |= 0x4; // Bus Master Enable
        PCIBus::pci_write(self.bus_id, self.slot_id, self.func_id,
                            PCIBus::REG_COMMAND, cmd);
        true
    }

    pub fn get_command(&self) -> Option<u16> {
        if !self.valid {
            return None;
        }
        Some((PCIBus::pci_read(self.bus_id, self.slot_id, self.func_id,
                            PCIBus::REG_COMMAND) & 0xFFFF) as u16)
    }
    pub fn get_status(&self) -> Option<u16> {
        if !self.valid {
            return None;
        }
        Some((PCIBus::pci_read(self.bus_id, self.slot_id, self.func_id,
                            PCIBus::REG_COMMAND) >> 16) as u16)
    }
    pub fn get_bar_address(&self, bar_index: usize) -> Option<(usize, usize)> {
        if bar_index >= 6 {
            return None;
        }
        let bar_val = self.bar[bar_index];
        if bar_val == 0 {
            return None; // Not implemented
        }
        if (bar_val & 0x1) == 0 {
            // To determine the amount of address space needed by a PCI device,
            // you must save the original value of the BAR, write a value of all
            // 1's to the register, then read it back. The amount of memory can
            // then be determined by masking the information bits, performing a
            // bitwise NOT, and incrementing the value by 1
            // 1) Save the base address of the bar:
            let base_addr = (bar_val & 0xFFFFFFF0) as usize;
            // 2) Write all 1's to the BAR:
            PCIBus::pci_write(self.bus_id, self.slot_id, self.func_id,
                        PCIBus::REG_BAR0 + bar_index as u8 * 4, 0xFFFFFFFF);
            // 3) Read the value back:
            let size_mask = PCIBus::pci_read(self.bus_id, self.slot_id, self.func_id,
                        PCIBus::REG_BAR0 + bar_index as u8 * 4) & 0xFFFFFFF0;
            // 4) Restore the original value:
            PCIBus::pci_write(self.bus_id, self.slot_id, self.func_id,
                        PCIBus::REG_BAR0 + bar_index as u8 * 4, bar_val);
            // 5) Calculate the size:
            let size = (!size_mask + 1) as usize;
            Some((base_addr, size))
        } else {
            None // I/O-mapped BAR
        }
    }
}

impl Display for PCIDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}.{} VID: {:X}, DID:{:X}, CLS:{:X}, SUB:{:X}, PIF:{:X}, \
                    IRQ:<L{},P{}>, BARs: [{:X}, {:X}, {:X}, {:X}, {:X}, {:X}]",
                self.bus_id, self.slot_id, self.func_id,
                self.vendor_id, self.device_id,
                self.class, self.sub_class, self.prog_if,
                self.irq_line, self.irq_pin,
                self.bar[0], self.bar[1], self.bar[2],
                self.bar[3], self.bar[4], self.bar[5]
        )
    }
}

pub static PCI_DEVICES: Spinlock<PCIBus> = Spinlock::new(PCIBus::new());

pub struct PCIBus {
    pub dev_lst:    Vec<PCIDevice>,
}

impl PCIBus {

    const PORT_CFG_ADDR:    u16 = 0xCF8;
    const PORT_CFG_DAT:     u16 = 0xCFC;

    const MAX_BUS_COUNT:    usize = 256;
    const MAX_SLOT_COUNT:   usize = 32;
    const MAX_FUNC_COUNT:   usize = 8;

    const ADDR_EN:          u32 = 0x80000000;
    const ADDR_BUS_LSHIFT:  u32 = 16;
    const ADDR_SLOT_LSHIFT: u32 = 11;
    const ADDR_FUNC_LSHIFT: u32 = 8;
    // Register Offset has to point to consecutive DWORDs, ie. bits 1:0 are 0b00
    const ADDR_REG_MASK:    u32 = 0xFC;

    const SLOT_NOT_EXISTS:  u16 = 0xFFFF; /* Sepcial VendorID */
    const REG_VENDOR:       u8 = 0; 
    const REG_DEVICE_ID:    u8 = 2;
    const REG_COMMAND:      u8 = 4;
    const REG_STATUS:       u8 = 6;
    const REG_REVISION_ID:  u8 = 8;
    const REG_PROG_IF:      u8 = 9;
    const REG_SUBCLASS:     u8 = 10;
    const REG_CLASS:        u8 = 11;
    const REG_HEADER_TYPE:  u8 = 14;
    
    const REG_BAR0:         u8 = 16;
    const REG_BAR1:         u8 = 20;
    const REG_BAR2:         u8 = 24;
    const REG_BAR3:         u8 = 28;
    const REG_BAR4:         u8 = 32;
    const REG_BAR5:         u8 = 36;

    const REG_IRQ_LINE:     u8 = 60;
    const REG_IRQ_PIN:      u8 = 61;

    pub const fn new() -> Self {
        Self {
            dev_lst: Vec::new()
        }
    }

    pub fn enumerate() -> usize {
        let mut pcibus = PCI_DEVICES.lock();
        for b in 0.. Self::MAX_BUS_COUNT {
            for s in 0..Self::MAX_SLOT_COUNT {
                if Self::pci_read(b as u8, s as u8, 0, Self::REG_VENDOR) as u16 
                                                    == Self::SLOT_NOT_EXISTS {
                    continue;
                }
                for f in 0..Self::MAX_FUNC_COUNT {
                    let vendor_id = Self::pci_read(b as u8, s as u8, f as u8,
                                                    Self::REG_VENDOR) as u16;
                    if vendor_id != Self::SLOT_NOT_EXISTS {
                        let mut new_dev = PCIDevice::new();
                        new_dev.load_from_pci(b as u8, s as u8, f as u8);
                        dbg!("PCI Device @{}\n", new_dev);
                        pcibus.dev_lst.push(new_dev);
                    }
                }
            }
        }
        pcibus.dev_lst.len()
    }

    pub fn release( _device: usize) {
        
    }

    pub fn pci_read(bus: u8, slot: u8, func: u8, reg_off: u8) -> u32 {
        let addr : u32;

        addr = Self::ADDR_EN |
            ((bus        as u32) << Self::ADDR_BUS_LSHIFT) |
            ((slot       as u32) << Self::ADDR_SLOT_LSHIFT) |
            ((func       as u32) << Self::ADDR_FUNC_LSHIFT) | 
            ((reg_off    as u32) & Self::ADDR_REG_MASK);
  
        x86_ioport_write(Self::PORT_CFG_ADDR, addr);
        return x86_ioport_read(Self::PORT_CFG_DAT);
    }

    pub fn pci_write(bus: u8, slot: u8, func: u8, reg_off: u8, val: u32) {
        let addr : u32;

        addr = Self::ADDR_EN |
            ((bus        as u32) << Self::ADDR_BUS_LSHIFT) |
            ((slot       as u32) << Self::ADDR_SLOT_LSHIFT) |
            ((func       as u32) << Self::ADDR_FUNC_LSHIFT) | 
            ((reg_off    as u32) & Self::ADDR_REG_MASK);
  
        x86_ioport_write(Self::PORT_CFG_ADDR, addr);
        return x86_ioport_write(Self::PORT_CFG_DAT, val);   
    }
}

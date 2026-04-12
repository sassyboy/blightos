//
// BlightOS kernel
//
// Advanced Host Controller Inerface (AHCI) Driver
//
// TODO: Support concurrent access & IO to drives (Request Queue!)
// TODO: Support Write Operations
// TODO: Support multi-entry scatter/gather data transfer
// TODO: Support SATAPI ? 
#![allow(dead_code)]
use core::hint::spin_loop;
use core::ptr::null_mut;
use core::time::Duration;
use alloc::collections::linked_list::LinkedList;
use alloc::{format, vec::*};
use crate::arch::{self, SystemTimer, SystemTimerTrait};
use crate::drivers::storage::{BusType, DISK_LIST, Disk, IOOperation, IORequest};
use crate::drivers::*;
use crate::drivers::pci::*;
use crate::mem::phys::*;
use crate::sched::Task;
use crate::util::*;
use crate::Error;
pub static AHCI_BUS: Spinlock<AHCIBus> = Spinlock::new(AHCIBus::new());

#[cfg(feature="debug_ahci")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[AHCI] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_ahci"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}


//
// AHCI Bus Driver
// Only one bus is enumerated and supported
//
pub struct AHCIBus {
    pci_dev:            PCIDevice,
    hba_reg_file:       *mut u32,
    pub drives:         Vec<AHCIDrive>,
    pub enumerated:     bool,
    pub irq_enabled:    bool
}

impl AHCIBus {

    const PORT_SSTS_IPM_ACTIVE:         u32 = 1;
    const PORT_SSTS_DET_PRESENT:        u32 = 3;
    const PORT_SIG_ATA:                 u32 = 0x00000101;
    const PORT_SIG_ATAPI:               u32 = 0xEB140101;
    const PORT_SIG_SEMB:                u32 = 0xC33C0101;
    const PORT_SIG_PM:                  u32 = 0x96690101;

    pub const fn new() -> Self {
        Self {
            pci_dev:        PCIDevice::new(),
            hba_reg_file:   null_mut(),
            drives:         Vec::new(),
            enumerated:     false,
            irq_enabled:    false
        }
    }

    pub fn enumerate() -> usize {
        // Find the first PCI Device [CLS: 1, SUB: 6]
        let pci_devs = PCI_DEVICES.lock();
        for dev in &pci_devs.dev_lst {
            if dev.class == 0x1 && dev.sub_class == 0x6 {
                let mut ahci = AHCI_BUS.lock();
                ahci.pci_dev = dev.clone();
                
                ahci.hba_reg_file = (dev.bar[5] & 0xFFFFF000) as *mut u32;
                
                // TODO Reset the controller
                //      (not needed as firmware has init'ed it)
                // driver.write_hba_reg(HBAReg::GHC, 1);
                // while driver.read_hba_reg(HBAReg::GHC) & 1 > 0 {
                //     spin_loop();
                // }
                // TODO Ensure PCI.CMD.BusMastering is set

                dbg!("ACHI CAP:{:X}, GHC:{:X}, IS:{:X}, PI:{:X}, VS:{:X} - \
                      PCI-STS/CMD: {:X}, IRQ-REG15: {}\n",
                    ahci.read_hba_reg(HBAReg::CAP),
                    ahci.read_hba_reg(HBAReg::GHC),
                    ahci.read_hba_reg(HBAReg::IS),
                    ahci.read_hba_reg(HBAReg::PI),
                    ahci.read_hba_reg(HBAReg::VS),
                    PCIBus::pci_read(dev.bus_id, dev.slot_id, dev.func_id, 0x4),
                    PCIBus::pci_read(dev.bus_id, dev.slot_id, dev.func_id, 60)
                );
                
                // Enumerate all disk drives connected to this AHCI bus.
                // It also sends an ATA IDENTIFY_DEVICE command to the drive and
                // register a Disk object with the kernel's storage module.
                ahci.enumerate_ports();

                // Enabled IRQ handling
                let gsi: u32;
                let vec: u8;
                if dev.irq_line == 0xFF {
                    // Need to extract the mapping from ACPI (requires AML!)
                    // OR 
                    // TODO: SUPPORT MSI/MSI-X interrupt handling!
                    // gsi = 16;
                    // vec = 16;
                    // let mut reg15 = PCIBus::pci_read(dev.bus_id, dev.slot_id,
                    //                                         dev.func_id, 60);
                    // reg15 &= 0xFFFFFF00;
                    // reg15 |= gsi;
                    // PCIBus::pci_write(dev.bus_id, dev.slot_id, dev.func_id, 60,
                    //                                                     reg15);
                    // reg15 = PCIBus::pci_read(dev.bus_id, dev.slot_id,
                    //                                         dev.func_id, 60);
                    // klog!("AHCI REDIRECTED IRQ to GSI{} VEC{}: PCI-60:{:X}\n",
                    //         gsi, vec, reg15);
                    
                    // crate::arch::irq_reroute(gsi, vec, true);
                    // crate::arch::isr_register(vec as u16, Self::irq_handler);
                    // ahci.irq_enabled = true;
                } else {
                    gsi = dev.irq_line as u32;
                    vec = dev.irq_line;
                    crate::arch::irq_reroute(gsi, vec, true);
                    crate::arch::isr_register(vec as u16, Self::irq_handler);
                    ahci.irq_enabled = true;
                }
                ahci.write_hba_reg(HBAReg::GHC,
                                    ahci.read_hba_reg(HBAReg::GHC) | 0x2);
                ahci.enumerated = true;                
                return 1;
            }
        }
        0
    }

    pub fn post_enum() {
        // Spawn a worker task per drive to handle its request
        // and completion queues
        let n = AHCI_BUS.lock().drives.len();
        for i in 0..n {
            Task::spawn_named(Self::drive_worker, i, 
                                                format!("AHCI-WORKER{}", i));
        }
    }

    pub fn release( _device: usize) {
    }


    fn read_hba_reg(&self, reg_index: HBAReg) -> u32 {
        unsafe {
            self.hba_reg_file.add(reg_index as usize).read_volatile()
        }
    }

    fn write_hba_reg(&self, reg_index: HBAReg, val: u32) {
        unsafe {
            self.hba_reg_file.add(reg_index as usize).write_volatile(val);
        }
    }

    fn read_port_reg(&self, port: usize, port_reg: HBAPortReg) -> u32 {
        let reg_index = (0x100 + port * 0x80) / 4 + port_reg as usize;
        unsafe {
            self.hba_reg_file.add(reg_index as usize).read_volatile()
        }
    }

    fn write_port_reg(&self, port: usize, port_reg: HBAPortReg, val: u32) {
        let reg_index = (0x100 + port * 0x80) / 4 + port_reg as usize;
        unsafe {
            self.hba_reg_file.add(reg_index as usize).write_volatile(val);
        }
    }

    fn get_drive_type(&self, port: usize) -> AHCIDriveType {
        let ssts = self.read_port_reg(port, HBAPortReg::SSTS);
        let ipm = (ssts >> 8) & 0xF;
        let det = ssts & 0xF; // SSTS.DET : Device Detection

        // The device must be powered and present
        if det != Self::PORT_SSTS_DET_PRESENT || 
            ipm != Self::PORT_SSTS_IPM_ACTIVE {
            return AHCIDriveType::Null;
        }

        let sig = self.read_port_reg(port, HBAPortReg::SIG);
        match sig {
            Self::PORT_SIG_ATAPI    => { return AHCIDriveType::SATAPI; },
            Self::PORT_SIG_SEMB     => { return AHCIDriveType::SEMB; },
            Self::PORT_SIG_PM       => { return AHCIDriveType::PM; },
            _                       => { return AHCIDriveType::SATA }
        }
        
    }

    fn enumerate_ports(&mut self) {
        let mut pi : u32 = self.read_hba_reg(HBAReg::PI);
        for i in 0..32 {
            if pi & 1 == 1 {
                let dev_type = self.get_drive_type(i);
                if dev_type == AHCIDriveType::SATA {
                    self.port_stop_cmds(i);
    
                    let mut drive = AHCIDrive::new();
                    drive.port = i;
                    drive.drv_type = dev_type;
                    // Port Memory (Command List, FIS, Command Table)
                    drive.init_memory();
                    // Set the Command List Base Pointer
                    self.write_port_reg(i, HBAPortReg::CLBU, 0);
                    self.write_port_reg(i, HBAPortReg::CLB, 
                                        drive.base_addr as u32);
                    // Set the FIS Base Pointer
                    self.write_port_reg(i, HBAPortReg::FBU, 0);
                    self.write_port_reg(i, HBAPortReg::FB,
                                (drive.base_addr + AHCIDrive::FB_OFFSET) as u32);
                    self.port_start_cmds(i);
                    // Enable Port's Interrupt Generation
                    self.write_port_reg(i, HBAPortReg::IE, 1);
                    // Identify the drive (Sector Size, #Sectors, Caps, etc.)
                    self.identify_device(&mut drive);
                    dbg!("Detected {:X?}\n", drive);
                    // Add the disk to the Kernel's Disk registry
                    let disk: Disk = Disk {
                        bus: BusType::AHCI,
                        bus_id: 0, // TODO support mutiple AHCI Buses
                        drive_id: self.drives.len() as u8,
                        part_id: 0,
                        sector_size: drive.sector_size,
                        sector_count: drive.sector_count.clone(),
                        issue_io: Self::issue_io
                    };
                    {
                        let mut dlst = DISK_LIST.lock();
                        dlst.push(disk);
                    }

                    // Move the drive object to this AHCI's drive list:
                    self.drives.push(drive);
                } else {
                    // No interrupts from ports we don't add as a drive
                    self.write_port_reg(i, HBAPortReg::IE, 0);
                }
            }
            pi = pi >> 1;
        }
    }

    fn port_stop_cmds(&self, port: usize) {
        // Clear Start (bit0) and FIS_Receive_Enable (bit4) bits of
        // the command register of the port
        let cmd = self.read_port_reg(port, HBAPortReg::CMDSTS);
        const MASK: u32 = !0x11;
        self.write_port_reg(port, HBAPortReg::CMDSTS, cmd & MASK);

        // Wait until both bits are cleared by the controller
        loop {
            if self.read_port_reg(port, HBAPortReg::CMDSTS) & 0x11 == 0 {
                break;
            }
            spin_loop();
        }
    }

    fn port_start_cmds(&self, port: usize) {
        // Wait until CR (Command List Running) is cleared
        loop {
            if self.read_port_reg(port, HBAPortReg::CMDSTS) & 0x8000 == 0 {
                break;
            }
            spin_loop();
        }

        // Set Start (bit0) and FIS_Receive_Enable (bit4)
        let cmd = self.read_port_reg(port, HBAPortReg::CMDSTS);
        self.write_port_reg(port, HBAPortReg::CMDSTS, cmd | 0x11);
    }

    fn port_wait_while_busy(&self, port: usize) {
        // TODO time out if the port is stuck
        // #define ATA_DEV_BUSY 0x80
        // #define ATA_DEV_DRQ 0x08
        loop {
            let tfd = self.read_port_reg(port, HBAPortReg::TFD);
            if tfd & 0x88 == 0 {
                break;
            }
        }
    }

    fn port_issue_cmd(&self, port: usize, cmd_index: usize){
        self.write_port_reg(port, HBAPortReg::CI, 1 << cmd_index);
    }

    fn port_wait_for_comp(&self, port: usize, cmd_index: usize) -> bool {
        for _i in 0..1000 {
            let ci = self.read_port_reg(port, HBAPortReg::CI);
            if ci & (1 << cmd_index) == 0 {
                break;
            }
            let is = self.read_port_reg(port, HBAPortReg::IS);
            if is & 0x40000000 > 0 { // TFES (Task File Error Status)
                dbg!("CMD COMP TFES Error - CI: {:X}, IS: {:X}, CMDSTS: {:X}, \
                    SERR: {:X}\n",
                    self.read_port_reg(port, HBAPortReg::CI),
                    self.read_port_reg(port, HBAPortReg::IS),
                    self.read_port_reg(port, HBAPortReg::CMDSTS),
                    self.read_port_reg(port, HBAPortReg::SERR)
                );
                return false;
            }
            arch::cpu_busywait(Duration::from_millis(1));
        }
        // Check TFES again
        let is = self.read_port_reg(port, HBAPortReg::IS);
        if is & 0x40000000 > 0 {
            return false;
        }
        dbg!("CMD COMP: CI: {:X}, IS: {:X}, CMDSTS: {:X}, SERR: {:X}\n",
            self.read_port_reg(port, HBAPortReg::CI),
            self.read_port_reg(port, HBAPortReg::IS),
            self.read_port_reg(port, HBAPortReg::CMDSTS),
            self.read_port_reg(port, HBAPortReg::SERR)
        );
        true
    }

    fn dump_cmd(&self, port: usize, cmd_index: usize) {
        let cmdlst_base: usize;
        let _fis_base :usize;
        cmdlst_base = (self.read_port_reg(port, HBAPortReg::CLB) as usize) |
                (self.read_port_reg(port, HBAPortReg::CLBU) as usize) << 32;
        _fis_base = (self.read_port_reg(port, HBAPortReg::FB) as usize) |
                (self.read_port_reg(port, HBAPortReg::FBU) as usize) << 32;
                
        dbg!("Port {}, Cmd {}: CMD List @ {:X}, FIS @{:X}, IE:{:X}\n",
            port, cmd_index, cmdlst_base, _fis_base,
            self.read_port_reg(port, HBAPortReg::IE)
        );
        
        let mut hdr = HBACommandHeader::new();
        hdr.decode(cmdlst_base + cmd_index * 32);
        dbg!("  CMD HEADER: {:X?}\n", hdr);
        for _i in 0..hdr.prdt_entries {
            let mut prd = HBAPhysicalRegionDescriptor::new();
            prd.decode(hdr.cmd_tbl_base as usize + 0x80);
            dbg!("  PRDT[{}] @ {:X}: {:X?}\n", _i,
                    hdr.cmd_tbl_base as usize + 0x80,
                    prd
            );
        }          
    }

    //
    // ATA_IDENTIFY_DEVICE
    //
    pub fn identify_device(&mut self, drv: &mut AHCIDrive) {
        // Allocate a buffer for receiving the IDENTIFY_DEVICE block (512 bytes)
        let mut dma = DMABuffer::new();
        dma.init(512, false);
        unsafe {
            let buffer_ptr: *mut u8 = dma.virt_addr as *mut u8;
            buffer_ptr.write_bytes(0xAA, 512);
        }
        // Clear pending interrupt
        self.write_port_reg(drv.port, HBAPortReg::IS, 0xFFFFFFFF);
        
        // Only use command 0 for now
        // 1) Prepare the command header in the command list
        let mut hdr = drv.read_cmd_header(0);
        hdr.flags = HBACommandHeader::FLAGS_CLEAR_BUSY;
        hdr.set_cmd_fis_len(HBAHostToDeviceFIS::FIS_LENGTH_DWORDS);
        hdr.prdt_entries = 1; // 1 scatter/gather descriptor is enough for 4KB
        drv.write_cmd_header(0, &hdr);
        // 2) Prepare the scatter/gather list
        drv.clear_cmd_table(0);
        let mut prd0    = HBAPhysicalRegionDescriptor::new();
        prd0.base_addr  = dma.phys_addr as u64;
        prd0.length     = 512;
        prd0.irq        = true;
        drv.write_cmd_prdt_entry(0, 0, &prd0);
        // 3) Prepare the Host2Device FIS part of the command (sector LBA, etc)
        let mut cfis    = HBAHostToDeviceFIS::new();
        cfis.ata_cmd    = 0xEC; // ATA_CMD_IDENTIFY 
        cfis.device     = HBAHostToDeviceFIS::DEV_MASTER;
        drv.write_cmd_cfis(0, &HBACommandFIS::Host2Device(cfis));

        dbg!("Sending ATA_CMD_IDENTIFY to port {}\n", drv.port);
        self.dump_cmd(drv.port, 0); // DEBUG

        self.port_wait_while_busy(drv.port);
        self.port_issue_cmd(drv.port, 0);
        if self.port_wait_for_comp(drv.port, 0) == true {
            // TODO figure this out
            // For some reason without this delay the buffer doesn't have the
            // data when running on real hardware.
            arch::cpu_busywait(Duration::from_millis(5));
            // Retrieve the following and update the device object:
            // From ATA/ATAPI-6 Specs
            // WORD INDEX: Meaning
            // -----------------
            // 49       If Bit 9 is set, the device supports LBA28.
            // 60–61	Total Sectors (28-bit)	For LBA devices
            // 83       If Bit 10 is set, the device supports LBA48 (>137GB)
            // 100–103	Total Sectors (48-bit)	For LBA48 supporting disks
            // 106	    Bit 12 set: sector size > 256 words & 117-118 are valid.
            // 117–118	Words per Logical Sector
            let ptr: *mut u16 = dma.virt_addr as *mut u16;
            unsafe {
                let w49 = ptr.add(83).read_volatile();
                let w83 = ptr.add(83).read_volatile();
                drv.cap = 0;
                if w49 & 0x100 > 0 {
                    drv.cap |= AHCIDrive::CAP_DMA_SUPPORTED;
                }
                if w83 & 0x400 > 0 {
                    drv.cap |= AHCIDrive::CAP_LBA48_SUPPORTED;
                    let w100 = ptr.add(100).read_volatile() as u64;
                    let w101 = ptr.add(101).read_volatile() as u64;
                    let w102 = ptr.add(102).read_volatile() as u64;
                    let w103 = ptr.add(103).read_volatile() as u64;
                    drv.sector_count = w100 | (w101 << 16) | (w102 << 32) | (w103 << 48);
                } else if w49 & 0x200 > 0 {
                    drv.cap |= AHCIDrive::CAP_LBA28_SUPPORTED;
                    let w60 = ptr.add(60).read_volatile() as u64;
                    let w61 = ptr.add(61).read_volatile() as u64;
                    drv.sector_count = w60 | (w61 << 16);
                } else {
                    dbg!("LBA/LBA48 not supported by drive @ port {}!\n",
                        drv.port);
                }
                let w106 = ptr.add(106).read_volatile();
                if w106 & 0x1000 > 0 {
                    let w117 = ptr.add(117).read_volatile() as u32;
                    let w118 = ptr.add(118).read_volatile() as u32;
                    drv.sector_size = (w117 | (w118 << 16)) * 2;
                } else {
                    drv.sector_size = 512;
                }
            }
            
        }
    }

    //
    // ATA_CMD_READ_DMA_EX
    //
    // Reads num_sectors (512 bytes each) from the disk drive from start_lba
    // and returns Option<cmd_number_used_for_io>
    //
    pub fn read_sectors(&self, drive: u8, start_lba: u64, num_sectors: u16,
                        buffer_addr: usize) -> Option<usize>  {
        if num_sectors > 8 {
            // Only support up to 4KB transfers for now
            return None;
        }
        // Clear pending interrupt
        let drv = &self.drives[drive as usize];
        self.write_port_reg(drv.port, HBAPortReg::IS, 0xFFFFFFFF);
        
        // Only use command 0 for now
        // 1) Prepare the command header in the command list
        let mut hdr = drv.read_cmd_header(0);
        hdr.flags |= HBACommandHeader::FLAGS_CLEAR_BUSY;
        hdr.set_cmd_fis_len(HBAHostToDeviceFIS::FIS_LENGTH_DWORDS);
        hdr.prdt_entries = 1; // 1 scatter/gather descriptor is enough for 4KB
        drv.write_cmd_header(0, &hdr);
        // 2) Prepare the scatter/gather list
        drv.clear_cmd_table(0);
        let mut prd0    = HBAPhysicalRegionDescriptor::new();
        prd0.base_addr  = buffer_addr as u64;
        prd0.length     = num_sectors as u32 * 512;
        prd0.irq        = true;
        drv.write_cmd_prdt_entry(0, 0, &prd0);
        // 3) Prepare the Host2Device FIS part of the command (sector LBA, etc)
        let mut cfis    = HBAHostToDeviceFIS::new();
        cfis.ata_cmd    = 0x25; // ATA_CMD_READ_DMA_EX
        cfis.device     = HBAHostToDeviceFIS::DEV_LBA_ADDRESSING;
        cfis.lba        = start_lba;
        cfis.sectors    = num_sectors;
        drv.write_cmd_cfis(0, &HBACommandFIS::Host2Device(cfis));

        dbg!("Sending ATA_CMD_READ_DMA_EX to port {}\n", drv.port);
        self.dump_cmd(drv.port, 0);

        self.port_wait_while_busy(drv.port);
        self.port_issue_cmd(drv.port, 0);
        
        Some(0) // TODO: use more command slots for concurrent access
    }

    //
    // Sends the IO to the drive.
    // Both the issue_io callback and the drive worker use this interface to
    // enqueue IO request onto the drive's command list
    //
    fn submit_io_command(ahci: &mut AHCIBus, drive_id: u8, mut iorq: IORequest) {
        let ret;
        // TODO - Each drive can handle up to CMD_TBL_COUNT commands at a time.
        //        Check the command list capacity to see if this request should
        //        be offloaded to the worker task by inserting to the drive's
        //        request queue.
        //        For now I assume that CMD[0] is always available and make the
        //        submission immediately here
        if iorq.op == IOOperation::Read {
            ret = ahci.read_sectors(drive_id,
                                    iorq.lba, iorq.sectors, iorq.buffer);
        } else {
            // TODO: support write
            ret = None;
        }
        iorq.ts_submitted = SystemTimer::current_timestamp();
        match ret {
            None    => {
                // Immediate failure -> add the completion object now
                if iorq.op == IOOperation::Read {
                    iorq.completion = Err(error!(ErrorCode::NotIssued));
                } else {
                    iorq.completion = Err(error!(ErrorCode::InvalidOp));
                }
                ahci.drives[drive_id as usize].completed_queue.push_back(iorq);
            },
            Some(cmd_num)   => {
                // Move the request to the submitted queue
                iorq.driver_priv = cmd_num; // For tracking in the IRQ
                ahci.drives[drive_id as usize].submitted_queue.push_back(iorq);
            }
        }
    }

    //
    // Issue IO Callback
    //
    fn issue_io(dsk: &Disk, iorq: &IORequest) {
        // Make a copy of the caller's request for the driver's internal use
        let mut cpyrq = iorq.clone();
        cpyrq.ts_issued = SystemTimer::current_timestamp();
        if dsk.bus != BusType::AHCI || dsk.bus_id != 0 {
            cpyrq.completion = Err(error!(ErrorCode::InvalidBus));
            (iorq.completion_cb)(cpyrq);
            return;
        }
        if iorq.lba + iorq.sectors as u64 > dsk.sector_count {
            cpyrq.completion = Err(error!(ErrorCode::OutOfBoundIO));
            (iorq.completion_cb)(cpyrq);
            return;
            
        }
        { // CRITICAL SECTION!
            let mut ahci = AHCI_BUS.lock();
            if dsk.drive_id as usize >= ahci.drives.len() {
                cpyrq.completion = Err(error!(ErrorCode::InvalidDrive));
                (iorq.completion_cb)(cpyrq);
                return;
            }
            cpyrq.waiter_tid = Task::current_tid();
            // offload all for now
            ahci.drives[dsk.drive_id as usize].request_queue.push_back(cpyrq);
            // Self::submit_io_command(&mut ahci, dsk.drive_id, cpyrq);
        }
        if iorq.sync == true {
            Task::block();
            // The drive's worker task will unblock the caller once the
            // the completion object is popped from the drive's completed queue.
        }
    }

    //
    // IRQ Handler
    //
    fn irq_handler(_irq: u16){
        
        let mut ahci = AHCI_BUS.lock();
        if ahci.read_hba_reg(HBAReg::IS) == 0 {
            // No pending events to process
            return;
        }
        for drv in 0..ahci.drives.len() {
            let port = ahci.drives[drv].port;
            let irq_status = ahci.read_port_reg(port, HBAPortReg::IS);
            if irq_status == 0 {
                // No interrupt status set?
                continue;
            }
            let cmd_issue = ahci.read_port_reg(port, HBAPortReg::CI);
            // Iterate over the submission queue and look for completion of
            // the corresponding command
            while ahci.drives[drv].submitted_queue.is_empty() == false {
                let mut ioreq = ahci.drives[drv].submitted_queue.pop_front()
                                            .expect("IO Submission Queue Bug!");
                let cmd_index = ioreq.driver_priv;
                if cmd_issue & (1 << cmd_index) == 0 {
                    ioreq.completion = Ok(
                        ahci.drives[drv].read_cmd_header(cmd_index).xfered_bytes
                        as usize
                    );
                    ioreq.ts_completed = SystemTimer::current_timestamp();
                    ahci.drives[drv].completed_queue.push_back(ioreq);
                }
                // TODO Figure out IO Errors
                //  let is = self.read_port_reg(port, HBAPortReg::IS);
                // if is & 0x40000000 > 0 { // TFES (Task File Error Status)
                //    return false;
                // }
            }
            
            // Processed all the events for this drive.
            ahci.write_port_reg(port, HBAPortReg::IS, 0xFFFFFFFF);
        }
        // Clear HBA's events
        ahci.write_hba_reg(HBAReg::IS, 0xFFFFFFFF);
    }
    //
    // Drive Worker Task
    //
    fn drive_worker(drive_id: usize) {
        // Select the last drive in the AHCI to work on
        dbg!("{} started on CPU {} - drive_id: {}\n", Task::name(),
                                            crate::arch::cpu_id(), drive_id);
        let call_irq_manually;
        {
            let ahci = AHCI_BUS.lock();
            call_irq_manually = !ahci.irq_enabled;
        }
        loop {
            // 1) Issue the offloaded IO requests
            let num_requests;
            {
                let ahci = AHCI_BUS.lock();
                num_requests = ahci.drives[drive_id as usize].request_queue.len();
            }
            for _i in 0..num_requests
            {
                let mut ahci = AHCI_BUS.lock();
                let iorq = ahci.drives[drive_id as usize].request_queue
                                    .pop_front().expect("IORequest Queue Bug");
                Self::submit_io_command(&mut ahci, drive_id as u8, iorq);
                // Todo - Handle the case that the cmd queue gets full again
            }

            // 2) Process completed IO
            if call_irq_manually {
                Self::irq_handler(0);
            }

            // 3) Call the completion handlers for completed IOs
            let num_completions;
            {
                let ahci = AHCI_BUS.lock();
                num_completions = ahci.drives[drive_id as usize].completed_queue.len();
            }
            for _i in 0..num_completions {
                let creq;
                {
                    let mut ahci = AHCI_BUS.lock();
                    creq = ahci.drives[drive_id as usize].completed_queue
                                .pop_front().expect("IO Completion Queue Bug!");
                }
                (creq.completion_cb)(creq);
                if creq.sync == true {
                    Task::wake(creq.waiter_tid);
                }
            }
        }
    }
}

#[repr(usize)]
enum HBAReg {
    // 0x00 - 0x2B, Generic Host Control
    CAP             = 0x0,  // 0x00: Host capability
    GHC             = 0x1,  // 0x04: Global host control
    IS              = 0x2,  // 0x08: Interrupt Status
    PI              = 0x3,  // 0x0C: Ports Implemented
    VS              = 0x4,  // 0x10: Version

}

#[repr(usize)]
enum HBAPortReg {
    CLB     = 0,    // 0x00 Command List Base Address
    CLBU    = 1,    // 0x04 Command List Base Address (Upper 32 bits)
    FB      = 2,    // 0x08 FIS Base Address
    FBU     = 3,    // 0x0C FIS Base Address (Upper 32 bits)
    IS      = 4,    // 0x10 Interrupt Status
    IE      = 5,    // 0x14 Interrupt Enable
    CMDSTS  = 6,    // 0x18 Command and Status
    TFD     = 8,    // 0x20 Task File Data
    SIG     = 9,    // 0x24 Signature
    SSTS    = 10,   // 0x28 SATA Status (SCR0: SStatus)
    SCTL    = 11,   // 0x2C SATA Control(SCR2: SControl)
    SERR    = 12,   // 0x30 SATA Error  (SCR1: SError)
    SACT    = 13,   // 0x34 SATA Active (SCR3: SActive)
    CI      = 14,   // 0x38 Command Issue
    SNTF    = 15,   // 0x3C SATA Notification
    FBS     = 16,   // 0x40 FIS-based Switching Control
    DEVSLP  = 17,   // 0x44 Device Sleep
}

#[derive(Debug, PartialEq, Eq)]
enum AHCIDriveType {
    Null,
    SATA,
    SEMB,
    PM,
    SATAPI
}

//
// Drive/Port connected to an AHCI
//
#[derive(Debug)]
pub struct AHCIDrive {
    port:           usize,
    drv_type:       AHCIDriveType,
    base_addr:      usize,
    // Requests to send to the drive when immediate submission is not possible
    request_queue:      LinkedList<IORequest>,
    // Requests that are sent to the drive but haven't completed yet
    submitted_queue:    LinkedList<IORequest>,
    // Requests that have been completed but their completion handler hasn't
    // been called yet.
    completed_queue:    LinkedList<IORequest>,
    

    // The following is initialized by the AHCI bus driver after this drive
    // is enumerated and initialized by sending an IDENTIFY_DEVICE ATA cmd
    cap:            u32,
    sector_size:    u32,
    sector_count:   u64,

}
impl AHCIDrive {

    const CAP_LBA28_SUPPORTED: u32 = 0x1;
    const CAP_LBA48_SUPPORTED: u32 = 0x2;
    const CAP_DMA_SUPPORTED:   u32 = 0x4;
    // Offsets into the port-specific memory (base_addr)
    // [0    to 1023]: Command List (Size: 1KB)
    // [1024 to 1279]: Received FIS (Size: 256 Bytes)
    // [1280 to ... ]: Command Table
    const CMD_LST_BASE: usize = 0;
    const FB_OFFSET:    usize = 1024;
    const CMD_TBL_BASE: usize = 1024+256;

    // There are up to 32 commands in the Command List, each of which points to
    // a Command Table, however, we only use the first 11 commands in the list
    // so that every AHCI port memory fits in a 4K page
    const CMD_TBL_COUNT:usize = 11;
    // Each Command Table is formatted as:
    // [0    to 0x40) Command FIS (up to 64 bytes)
    // [0x40 to 0x50) ATAPI Command (12 or 16 bytes)
    // [0x50 to 0x80) RSVD
    // [0x80 to ....] Physical Region Descriptor Table (aka scatter/gather list)
    // For now, this driver only support 8 PRDT entries per Command Table. Each
    // entry is 16 bytes
    const PRDT_ENTRIES_PER_CTBL: usize = 8;
    const CMD_TBL_SIZE: usize = 0x80 + Self::PRDT_ENTRIES_PER_CTBL * 16;

    const MEM_SIZE: usize = Self::CMD_TBL_BASE + 
                            Self::CMD_TBL_COUNT * Self::CMD_TBL_SIZE;

    const fn new() -> Self{
        Self {
            port:           0,
            drv_type:       AHCIDriveType::Null,
            base_addr:      0,
            cap:            0,
            sector_count:   0,
            sector_size:    0,
            request_queue:  LinkedList::new(),
            completed_queue:LinkedList::new(),
            submitted_queue:LinkedList::new()
        }
    }

    fn cmd_hdr_base(&self, cmd_index: usize) -> usize {
        self.base_addr + cmd_index * 32
    }

    fn cmd_table_base(&self, cmd_index: usize) -> usize {
        self.base_addr + Self::CMD_TBL_BASE + cmd_index * Self::CMD_TBL_SIZE
    }

    fn init_memory(&mut self) {
        self.base_addr = PhysMem::alloc().expect("Out of memory");
        let mem : *mut u8 = self.base_addr as *mut u8;
        // Zero out the port memory
        unsafe {
            mem.write_bytes(0, PHY_FRAME_SIZE);
        }
        // Set up the first CMD_TBL_COUNT Command -> Command Table items
        for i in 0..Self::CMD_TBL_COUNT {
            let mut cmd = HBACommandHeader::new();
            cmd.prdt_entries = Self::PRDT_ENTRIES_PER_CTBL as u16;
            cmd.cmd_tbl_base = self.cmd_table_base(i) as u64;
            cmd.encode(self.cmd_hdr_base(i));
        }
    }

    fn clear_cmd_table(&self, cmd_index: usize) {
        let ptr : *mut u8 = self.cmd_table_base(cmd_index) as *mut u8;
        unsafe {
            for _i in 0..Self::CMD_TBL_SIZE {
                ptr.write_volatile(0);
            }
        }
    }

    fn read_cmd_header(&self, cmd_index: usize) -> HBACommandHeader {
        let mut cmd = HBACommandHeader::new();
        cmd.decode(self.cmd_hdr_base(cmd_index));
        cmd
    }

    fn write_cmd_header(&self, cmd_index: usize, cmd: &HBACommandHeader) {
        cmd.encode(self.cmd_hdr_base(cmd_index));
    }

    fn write_cmd_cfis(&self, cmd_index: usize, cfis: &HBACommandFIS) {
        match cfis {
            HBACommandFIS::Host2Device(h2b) => {
                h2b.encode(self.cmd_table_base(cmd_index));
            }
        }
    }

    fn write_cmd_prdt_entry(&self, cmd_index: usize, entry_index: usize,
                                            prd: &HBAPhysicalRegionDescriptor) {
        prd.encode(self.cmd_table_base(cmd_index) + 0x80 + entry_index * 16);
    }
}

impl Drop for AHCIDrive {
    fn drop(&mut self) {
        if self.base_addr != 0 {
            PhysMem::free(self.base_addr);
        }
    }
}

//
// HBACommandHeader
// Each Port's CLB register points to an array of (up to 32) this structure
// A command header describes flags for the command and points to a PRDT base,
// i.e., a Physical Region Descriptor Table (scatter/gather)
//

#[derive(Debug)]
struct HBACommandHeader {
    pub flags:       u16, // DW0[15.. 0]: PMP, C, B, R, P, W, A, CFL
    pub prdt_entries:u16, // DW0[31..16]: # entries in the scatter/gather table
    pub xfered_bytes:u32, // DW1: # bytes transferred between the host & device
    pub cmd_tbl_base:u64, // DW2 (lower 32) and DW3 (upper 32)
    // rsvd:           [u32; 4]
}
impl HBACommandHeader {
    const FLAGS_ATAPI:        u16 = 0x20;
    const FLAGS_WRITE:        u16 = 0x40;
    const FLAGS_PREFETCH:     u16 = 0x80;
    const FLAGS_RESET:        u16 = 0x100;
    // Host clears PxCI.CI and PxTFD.STS.BSY after sending the FIS and receiving
    // R_OK if the CLEAR_BUSY flag is set
    const FLAGS_CLEAR_BUSY:   u16 = 0x400; 

    pub const fn new() -> Self {
        Self { 
            flags: 0,
            prdt_entries: 0,
            xfered_bytes: 0,
            cmd_tbl_base: 0,
        }
    }

    pub fn set_cmd_fis_len(&mut self, num_dwords: u8) {
        self.flags = (self.flags & 0xFFE0) | (num_dwords & 0x1F) as u16;
    }

}
impl DeviceStruct for HBACommandHeader {
    
    fn encode(&self, dest_addr: usize) {
        let ptr: *mut u32 = dest_addr as *mut u32;
        unsafe {
            ptr.add(0).write_volatile((self.prdt_entries as u32) << 16 |
                                        self.flags as u32);
            ptr.add(1).write_volatile(self.xfered_bytes);
            ptr.add(2).write_volatile((self.cmd_tbl_base & 0xFFFFFF80) as u32);
            ptr.add(3).write_volatile((self.cmd_tbl_base >> 32) as u32);
        }
    }

    fn decode(&mut self, src_addr: usize) {
        let ptr: *mut u32 = src_addr as *mut u32;
        unsafe {
            let dw0 = ptr.read_volatile();
            self.flags = (dw0 & 0xFFFF) as u16;
            self.prdt_entries = (dw0 >> 16) as u16;
            self.xfered_bytes = ptr.add(1).read_volatile();
            self.cmd_tbl_base = (ptr.add(2).read_volatile() as u64) |
                                (ptr.add(3).read_volatile() as u64) << 32;
        }
    }
}

//
// An scatter/gather descriptor in the PRDT of a command
//
#[derive(Debug)]
struct HBAPhysicalRegionDescriptor {
    base_addr:  u64,    // Base physical address of the data buffer
    length:     u32,    // Lenght of the data buffer
    irq:        bool,   // DW0.bit31: Generate an interrupt on completion
}
impl HBAPhysicalRegionDescriptor {
    pub const fn new() -> Self {
        Self {
            base_addr:  0,
            length:     0,
            irq:        false
        }
    }
}
impl DeviceStruct for HBAPhysicalRegionDescriptor {
    fn encode(&self, dest_addr: usize) {
        let ptr : *mut u32 = dest_addr as *mut u32;
        unsafe {
            ptr.add(0).write_volatile((self.base_addr & 0xFFFFFFFE) as u32);
            ptr.add(1).write_volatile((self.base_addr >> 32) as u32);
            if self.irq == true {
                ptr.add(3).write_volatile((self.length -1) as u32 | 0x80000000);
            } else {
                ptr.add(3).write_volatile((self.length -1) as u32);
            }
        }
    }
    fn decode(&mut self, src_addr: usize) {
        let ptr : *mut u32 = src_addr as *mut u32;
        unsafe {
            self.base_addr = ptr.add(0).read_volatile() as u64 |
                            (ptr.add(1).read_volatile() as u64) << 32;
            let dw3 = ptr.add(3).read_volatile();
            self.irq = dw3 & 0x80000000 > 0;
            self.length = (dw3 & 0x3FFFFF) + 1;   
        }
    }

}

//
// Command FIS (Frame Information Structure)
//
enum HBACommandFIS {
    Host2Device(HBAHostToDeviceFIS),
}

struct HBAHostToDeviceFIS {
    lba:        u64,
    sectors:    u16,
    ata_cmd:    u8,
    pm_port:    u8,
    device:     u8, // Disk Head #: 64:LBA-Addressing/ 0: Master
}
impl HBAHostToDeviceFIS {
    pub const FIS_LENGTH_DWORDS:    u8 = 5;
    pub const DEV_LBA_ADDRESSING:   u8 = 0x40;
    pub const DEV_MASTER:           u8 = 0;
    pub const fn new() -> Self {
        Self { lba: 0, sectors: 0, pm_port: 0, ata_cmd: 0, device: 0 }
    }
}
impl DeviceStruct for HBAHostToDeviceFIS {
    fn encode(&self, dest_addr: usize) {
        let ptr : *mut u32 = dest_addr as *mut u32;
        // See SATA Rev2.6 - Section 10.3.4
        // DW0: Features|ATA_Command|Cmd_Update+PMPort|FIS_TYPE_REG_H2D (27)
        let dw0 =   ((self.ata_cmd as u32)       << 16) | 
                    ((0x80 | self.pm_port as u32) << 8) | 0x27 as u32;
        // DW1: Device/Head | LBA High | LBA Mid | LBA Low
        let dw1 =   (self.device as u32) << 24 | (self.lba & 0xFFFFFF) as u32;
        // DW2: FeaturesExp | LBA_Exp High | LBA_Exp Mid | LBA_Exp Low
        let dw2 =   ((self.lba >> 24) & 0xFFFFFF) as u32;
        // DW3: Control | RSVD(0) | Sector Count High | Sector Count Low
        let dw3 =   self.sectors as u32;
        // DW4: RSVD(0) | RSVD(0) | RSVD(0) | RSVD(0)
        unsafe {
            ptr.add(0).write_volatile(dw0);
            ptr.add(1).write_volatile(dw1);
            ptr.add(2).write_volatile(dw2);
            ptr.add(3).write_volatile(dw3);
            ptr.add(4).write_volatile(0);
        }
    }
    fn decode(&mut self, src_addr: usize) {
        let ptr : *mut u32 = src_addr as *mut u32;
        unsafe {
            let dw0 = ptr.add(0).read_volatile();
            let dw1 = ptr.add(1).read_volatile();
            self.ata_cmd = ((dw0 >> 16) & 0xFF) as u8;
            self.pm_port = ((dw0 >> 8 ) & 0x0F) as u8;
            self.device = (dw1 >> 24) as u8;
            self.lba = (dw1 & 0xFFFFFF) as u64;
            self.lba|= ((ptr.add(2).read_volatile() & 0xFFFFFF) as u64) << 32;
            self.sectors = (ptr.add(3).read_volatile() & 0xFFFF) as u16;
        }

    }
}

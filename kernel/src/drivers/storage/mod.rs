//
// BlightOS Kernel
//
// Mass Storage Interface
//

use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering::Relaxed;
use alloc::{collections::btree_map::BTreeMap, vec::Vec};
use crate::{mem::phys::{palloc, pfree}, util::Spinlock};

pub mod ahci;

// Drivers register their enumerated disk drives by adding a Disk object to
// this Vector.
// The rest of the kernel detect and obtain copies of Disk objects via this
// vector as well.
pub static DISK_LIST: Spinlock<Vec<Disk>> = Spinlock::new(Vec::new());

#[derive(Clone, Copy, Debug)]
pub struct Disk {
    // Host/Drive Identification
    pub bus:            BusType,
    pub bus_id:         u8,
    pub drive_id:       u8,
    pub part_id:        u16,
    // Geometry
    pub sector_size:     u32,
    pub sector_count:    u64,
    // IO callbacks
    pub issue_io:       fn(&Disk, &IORequest) 
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusType {
    AHCI,
    None
}
#[derive(Clone, Copy, Debug)]
pub struct IORequest {
    pub req_id:         u64,    // Private data for the caller for IO tracking
    pub op:             IOOperation,
    pub sync:           bool,
    pub lba:            u64,
    pub sectors:        u16,
    pub buffer:         usize, // physical address - TODO replace with a list

    pub waiter_tid:     usize, // Set to the TID of the task that issued the IO
    pub completion_code:IOCompletion,
    pub completion_cb:  fn(IORequest),

    pub driver_priv:    usize,
    // statistics
    // Timestamped when Disk::issue_io() called
    pub ts_issued:      u64,
    // Timestamped when the IO is submitted to the drive
    pub ts_submitted:   u64,
    // Timestamped when the drivers finds out about the completion of the IO
    pub ts_completed:   u64,
}
impl IORequest {
    pub const fn new() -> Self {
        Self {
            req_id:         0,
            op:             IOOperation::Read,
            sync:           true,
            lba:            0,
            sectors:        0,
            buffer:         0,
            waiter_tid:     0,
            completion_code:IOCompletion::NotIssued,
            completion_cb:  (|_: IORequest| {}),
            driver_priv:    0,
            ts_issued:      0,
            ts_submitted:   0,
            ts_completed:   0,
        }
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IOOperation {
    Read,
    Write
}
#[derive(Clone, Copy, Debug)]
pub enum IOCompletion {
    Successful(usize),
    NotIssued,
    InvalidOp,
    InvalidBus,
    InvalidDrive,
    InvalidPath,
    InvalidHandle,
    InvalidBuffer,
    OutOfBoundIO,
}

///
/// Utilities
/// 

pub fn num_disks() -> usize {
    DISK_LIST.lock().len()
}

// Obtains a copy of a disk object
pub fn get_disk(disk_index: usize) -> Option<Disk> {
    let dlist = DISK_LIST.lock();
    if disk_index >= dlist.len() {
        return None;
    }
    Some(dlist[disk_index].clone())
}

static REQID_IOREQ_MAP: Spinlock<BTreeMap<usize, IORequest>> =
                        Spinlock::new(BTreeMap::new());
static NEXT_REQ_ID:     AtomicU64 = AtomicU64::new(0);
// Issue the IO request in a synchronous manner (blocks the caller), and handles
// the completion and returns another IORequest object once completed
pub fn submit_sync_io(disk: &Disk, ioreq: &mut IORequest) -> Option<IORequest> {
    ioreq.req_id = NEXT_REQ_ID.fetch_add(1, Relaxed);
    ioreq.completion_cb = | cioreq: IORequest | {
        REQID_IOREQ_MAP.lock().insert(cioreq.req_id as usize, cioreq);
    };
    (disk.issue_io)(disk, &ioreq);
    REQID_IOREQ_MAP.lock().remove(&(ioreq.req_id as usize))
}


pub enum DiskAddress {
    LBA {
        lba:            u64,
        block_offset:   u32
    },
    ByteAddr {
        addr:           u64
    }
}

///
/// BufferedDiskIO
/// A convenience struct that performs IO against a Disk and maintains an
/// internal memory buffer
/// 
pub struct BufferedDiskIO {
    // Cache
    buffer:         usize,
    first_lba:      u64,
    sec_count:      u64,
    //
    disk:           Disk
    //
}

impl BufferedDiskIO {

    const CACHE_SIZE: u64 = 4096;
    pub fn new(disk_index: usize) -> Option<Self> {
        match get_disk(disk_index) {
            Some(d) => Some(Self {
                buffer:     0,
                first_lba:  0,
                sec_count:  0,
                disk:       d
            }),
            None => None
        }
    }

    fn fetch_from(&mut self, addr: DiskAddress) {
        if self.buffer == 0 {
            // allocate the buffer on the first IO call
            self.buffer = palloc().expect("Out of memory");
        }

        match addr {
            DiskAddress::LBA { lba, .. }    => {
                self.first_lba = lba;

            },
            DiskAddress::ByteAddr { addr }      => {
                self.first_lba = addr / self.disk.sector_size as u64;
            }
        }
        self.sec_count = Self::CACHE_SIZE / self.disk.sector_size as u64;

        let bufp: *mut u8 = self.buffer as *mut u8;
        unsafe { bufp.write_bytes(0, 4096); }

        let mut ioreq = IORequest::new();
        ioreq.req_id =          0;
        ioreq.sync =            true;
        ioreq.op =              IOOperation::Read;
        ioreq.lba =             self.first_lba;
        ioreq.sectors =         self.sec_count as u16;
        ioreq.buffer =          self.buffer;
        (self.disk.issue_io)(&self.disk, &ioreq);
        // klog!("  FETCHED {} sectors from lba {}\n", self.sec_count, self.first_lba);
        // dump_memory_columns(self.buffer, 5, 5);
    
    }

    // Returns a virtual memory address (from the internal buffer) where the
    // data can be found. The caller can cast the address and read from it
    pub fn read(&mut self, addr: DiskAddress, bytes: usize)
        -> (usize, IOCompletion)
    {
        // Is the requested range covered by the cached IO?
        let first_addr = self.first_lba * self.disk.sector_size as u64;
        let last_addr  = first_addr + 
                         self.sec_count * self.disk.sector_size as u64;

        let disk_byte_addr;
        match addr {
            DiskAddress::ByteAddr { addr }  => {
                disk_byte_addr = addr;
            },
            DiskAddress::LBA { lba, block_offset } => {
                disk_byte_addr = lba * self.disk.sector_size as u64 +
                                block_offset as u64;
            }
        }

        // klog!("READ {} bytes from byte_addr: {} (secsz:{}), cache: [{} to {}]\n",
        //             bytes, disk_byte_addr, self.disk.sector_size,
        //             first_addr, last_addr);

        if !(disk_byte_addr >= first_addr &&
             (disk_byte_addr + bytes as u64) < last_addr ) {
            self.fetch_from(DiskAddress::ByteAddr { addr: disk_byte_addr });
        }


        let first_addr = self.first_lba * self.disk.sector_size as u64;
        let bufsz = self.sec_count * self.disk.sector_size as u64;
        let off = (disk_byte_addr - first_addr) as u64;
        if bytes as u64 > bufsz - off {
            // TODO: support requests larger than buffer - multiple fetches
            return (0, IOCompletion::NotIssued);
        }

        (self.buffer + off as usize, IOCompletion::Successful(bytes))
    }
}

impl Drop for BufferedDiskIO {
    fn drop(&mut self) {
        if self.buffer != 0 {
            pfree(self.buffer);
        }
    }
}

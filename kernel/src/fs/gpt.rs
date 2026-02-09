//
// BlightOS Kernel
//
// GUID Partition Table
//

use crate::drivers::storage::*;
use crate::fs::fat::FATVolume;
use crate::util::*;

#[repr(C, packed)]
struct GPTHeader {
    sig:                [u8; 8],
    revision:           u32,
    hdr_size:           u32,
    hdr_crc32:          u32,
    rsvd0:              u32,
    my_lba:             u64, // LBA location of this GPT Header.
    alt_lba:            u64, // LBA address of the alternate GPT Header
    first_usable_lba:   u64,
    last_usable_lba:    u64,
    disk_guid:          u128,
    gpt_start_lba:      u64, // Starting LBA of the GPT entry array; most often Sector 2 for the Primary Partition. 
    gpt_entry_count:    u32, // # of Partition Entries in the GPT Entry array. Most often 128.
    gpt_entry_size:     u32, // Size of each GPT entry in bytes. Should be set to a value of: 128 x 2n 
    gpt_crc32:          u32, // Starts at gpt_start_lba and is computed over a byte length of gpt_entry_count * gpt_entry_size
}

#[repr(C, packed)]
struct GPTEntry {
    part_type_guid:     u128, // Unique ID that defines the purpose and type of this Partition. A value of zero defines that this partition entry is not being used. every file system should have its own unique ID.
    part_unique_guid:   u128, // A GUID that is unique for every partition entry.
    part_start_lba:     u64, // Starting LBA [First Sector] of the partition defined by this entry.
    part_last_lba:      u64, // Ending LBA [Last Sector] of the partition defined by this entry.
    rsvd:               u64, // zero
    part_name:          [u8;72], // Null-terminated string
}

pub fn enumerate_partitions(disk_index: usize) -> usize {
    // Read LBA[1] - GPT Header if any
    let mut num_parts: usize = 0;
    let mut bdio = BufferedDiskIO::new(disk_index)
                    .expect("BufferedDiskIO::new() failed!");

    let hdr : GPTHeader;
    let (bdio_buf, comp) = bdio.read(DiskAddress::LBA { lba: 1, block_offset: 0 },
                                size_of::<GPTHeader>());
    if let IOCompletion::Successful(sz) = comp {
        if sz < size_of::<GPTHeader>() {
            return 0;
        }
    } else {
        return 0;
    }
    unsafe {
        hdr = (bdio_buf as *mut GPTHeader).read_volatile();
    }

    let disk = get_disk(disk_index).expect("Disk not found!");
    if str::from_utf8(&hdr.sig) == str::from_utf8(b"EFI PART") {

        // klog!("* Disk {:?}{}.{} uses GPT @ lba {} crc32: {:X} sectors: {}\n",
        //         disk.bus, disk.bus_id, disk.drive_id,
        //         hdr.gpt_start_lba as u64, hdr.hdr_crc32 as u32,
        //         disk.sector_count
        // );

        let entries =       hdr.gpt_entry_count;
        let mut cur_off =   hdr.gpt_start_lba * disk.sector_size as u64;
        for _i in 0..entries {
            // Read the entry
            let part_ent : GPTEntry;
            let (bdio_buf, _) = 
                    bdio.read(DiskAddress::ByteAddr { addr: cur_off },
                                                        size_of::<GPTHeader>());
            unsafe {
                part_ent = (bdio_buf as *mut GPTEntry).read_volatile();
            }

            if part_ent.part_type_guid != 0 {
                // FAT12/16/32
                if FATVolume::mount(disk_index, num_parts,
                                    part_ent.part_start_lba,
                                    part_ent.part_last_lba) {
                    // FATVolume::mount registers a mount point with the VFS
                    num_parts += 1;
                }
                cur_off += hdr.gpt_entry_size as u64;
            }
        }
    } else {
        klog!("  Disk {:?}{}.{} doesn't use GPT\n",
                disk.bus, disk.bus_id, disk.drive_id);
    }
    num_parts
}
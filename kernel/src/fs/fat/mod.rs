//
// BlightOS kernel
//
// FAT12/16/32 Filesystem
//
// Basic idea of the the File Allocation Table (FAT):
// The volume is divided into two important areas (apart from BootSector, ...)
// 1) A file allocation table that indicates how the disk is allocated to files
// 2) A data area, i.e., the file/directory data
//
// The data area is divided into blocks of a certain number of sectors called
// Cluster (i.e. the Allocation Unit) and the data area is managed in this unit.
// Each item of FAT is associated with each cluster in the data area and the FAT
// value indicates the state of the corresponding cluster.
//
// However, the top two FAT items, FAT[0] and FAT[1], are reserved and not
// associated with any cluster.
// The third FAT item, FAT[2], is the item associated with the first cluster of
// data area and the valid cluster number starts at 2
//
#![allow(dead_code)]

use core::fmt::Debug;
use core::slice;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::Relaxed;
use alloc::collections::btree_map::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::fs::{DirectoryEntry, FileOperation, MountPoint};
use crate::util::*;

use crate::drivers::storage::*;
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FATKind {
    FAT12,
    FAT16,
    FAT32,
    Unknown
}
#[derive(Debug)]
enum FATEntry {
    Free,
    Reserved,
    InUse(u32), // Value is the link to the next
    BadCluster,
    EndOfChain, // In use & end of the cluster chain
    Invalid
}

#[repr(C, packed)]
pub struct DirEntry {
    // Allowable charactes for the SFN are ASCII alphanumerics, some ASCII marks
    // ($%'-_@~`!(){}^#&) and extended characters (\x80 - \xFF)
    name:               [u8; 11], // SFN: Short File Name[8].Ext[3]
    attr:               u8,
    nt_res:             u8, // Optional flag for the case of short file name
    create_time_tenth:  u8,
    create_time:        u16,
    create_date:        u16,
    access_date:        u16,
    first_cluster_hi:   u16, // Uper part of the 1st cluster # (0 for FAT12/16)
    write_time:         u16,
    write_date:         u16,
    first_cluster_lo:   u16,
    file_size:          u32, // Size of the file in bytes. 0 for directoryes
}

impl DirEntry {
    const ATTR_READ_ONLY:       u8 = 0x01;
    const ATTR_HIDDEN:          u8 = 0x02;
    const ATTR_SYSTEM:          u8 = 0x04;
    const ATTR_VOLUME_ID:       u8 = 0x08;
    const ATTR_DIRECTORY:       u8 = 0x10;
    const ATTR_ARCHIVE:         u8 = 0x20; // File changed since last backup
    const ATTR_LONG_FILE_NAME:  u8 = 0x0F;

    const NT_RES_BODY:  u8 = 0x08; // SFN name characters all lower case
    const NT_RES_EXT:   u8 = 0x10; // SFN extension characters all lower case

    const NAME_0_FREE_ENTRY:    u8 = 0xE5;  // Free DIR Entry if name[0]=0xE5
    const NAME_0_LAST_ENTRY:    u8 = 0x0;   // Last DIR Entry (also free)

    pub fn sfn_match(&self, sfn: &str) -> bool {
        // Examples:
        // "SRC        " == "src" or "SRC  ", etc
        // "prog    exe" == "prog.exe" or "PROG.EXE" or "PROG    EXE"
        // "bootx64 efi" == "bootx64.efi"
        // "..         " == ".."
        let mut j = 0;      // index into the given sfn
        for i in 0..11 {    // index into the current entry's name
            let src_c = self.name[i].to_ascii_uppercase();
            let dst_c;
            if j >= sfn.len() {
                dst_c = b' '; // Pad shorter SFNs with spaces
            } else {
                dst_c = sfn.as_bytes()[j].to_ascii_uppercase();
            }
            if src_c != dst_c {
                if src_c == b' ' && dst_c == b'.' {
                    if i < 10 && self.name[i+1] != b' ' {
                        j += 1;
                    }
                    continue;
                }
                else {
                    return false;
                }
            }
            j += 1;
        }
        true
    }
    
    pub fn name(&self) -> String {
        String::from_utf8(self.name.to_vec()).unwrap()
    }

    pub fn size_bytes(&self) -> usize {
        self.file_size as usize
    }
}
impl Debug for DirEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let fsize = self.file_size;
        write!(f, "{} - attr: 0x{:X} size: {} 1st cluster {}", 
            str::from_utf8(&self.name).unwrap(),
            self.attr, fsize,
            (self.first_cluster_hi as u32) << 16 | (self.first_cluster_lo as u32)
        )
    }
}

enum AUMap {
    FAT1216Root {start_sec: u32, entry_count: u16},
    FATFile {cluster_list: Vec<u32>}
}

pub struct FATFile {
    dir_entry:          DirEntry,
    au_map:             AUMap,
    disk_index:         usize,
    first_lba:          u64,
    sector_size:        u16,
    cluster_sectors:    u8,
    cluster_size:       u32,
    data_start_sector:  u32,
}
impl FATFile {

    pub fn from_dir_entry(vol: &FATVolume, dir: DirEntry) -> Self {
        let au_map: AUMap;
        // Load and cache all the mappings
        let mut cur_cluster = (dir.first_cluster_hi as u32) << 16 | 
                                dir.first_cluster_lo as u32;
        if cur_cluster == 0 {
            // FAT12/16 Root Director -> Sectors!
            au_map = AUMap::FAT1216Root {
                start_sec:      vol.root_start_sector,
                entry_count:    vol.root_entry_count
            }
        } else {
            // FAT12/16/32 Files or Directories
            let mut bdio = BufferedDiskIO::new(vol.disk_index).unwrap();
            let mut clst: Vec<u32> = Vec::new();
            loop {
                clst.push(cur_cluster);
                let fat_ent = vol.load_fat_entry(&mut bdio, cur_cluster as usize);
                if let FATEntry::InUse(next_cluster) = fat_ent {
                    cur_cluster = next_cluster;
                } else {
                    break;
                }
            }
            
            au_map = AUMap::FATFile { cluster_list: clst };
        }
        Self {
            dir_entry:          dir,
            au_map:             au_map,
            disk_index:         vol.disk_index,
            first_lba:          vol.first_lba,
            sector_size:        vol.sector_size,
            cluster_sectors:    vol.cluster_sectors,
            cluster_size:       vol.cluster_size,
            data_start_sector:  vol.data_start_sector
        }
    }

    pub fn from_root(vol: &FATVolume) -> Self {
        let dir_entry = DirEntry {
            access_date:    0,
            attr:           DirEntry::ATTR_DIRECTORY,
            create_date:    0,
            create_time:    0,
            create_time_tenth: 0,
            file_size:  0,
            first_cluster_hi: (vol.root32_cluster0 >> 16) as u16,
            first_cluster_lo: (vol.root32_cluster0 & 0xFFFF) as u16,
            name: *b"<ROOT_DIRE>",
            nt_res: 0,
            write_date: 0,
            write_time: 0
        };
        FATFile::from_dir_entry(vol, dir_entry)
    }

    pub fn from_path(vol: &FATVolume, in_path: &str) -> Option<Self>{
        let mut path = in_path;
        // skip the initial / if the path starts with one
        if path.starts_with("/") {
            path = &path[1..];
        }

        let nodes = path.split('/');
        // klog!("nodes = {:?}\n", nodes);
        let mut cur_file = FATFile::from_root(vol);
        for fname in nodes {
            if fname.is_empty() {
                break;
            }
            if cur_file.is_directory() {
                // klog!("path traversal: {:?}\n", cur_file);
                // Look for the entry named fname (TODO - Support Long Names)
                if let Some(dent) = cur_file.find_child_by_name(fname) {
                    // klog!("  FOUND {:?}\n", dent);
                    cur_file = FATFile::from_dir_entry(vol, dent);
                } else {
                    return None;
                }
            }
        }
        Some(cur_file)
    }

    pub fn find_child_by_name(&self, sfn: &str) -> Option<DirEntry> {
        if self.is_directory() == false {
            return None;
        }

        let mut bdio = BufferedDiskIO::new(self.disk_index).unwrap();
        let mut file_off = 0;
        loop {
            let (memaddr, ioc) = self.read_int(&mut bdio, file_off, 
                                                        size_of::<DirEntry>());
            if let IOCompletion::Successful(_) = ioc{
                unsafe {
                    let dir_ent = (memaddr as *const DirEntry).read();
                    if dir_ent.name[0] != DirEntry::NAME_0_FREE_ENTRY &&
                       dir_ent.name[0] != DirEntry::NAME_0_LAST_ENTRY &&
                       dir_ent.attr & DirEntry::ATTR_LONG_FILE_NAME == 0 && 
                       dir_ent.sfn_match(sfn) == true
                    {
                        // Found it!
                        return Some(dir_ent);
                    } else if dir_ent.name[0] == DirEntry::NAME_0_LAST_ENTRY {
                        break;
                    }
                }
                file_off += size_of::<DirEntry>();
            } else {
                // Hit the end of the directory content 
                break;
            }    
        }
        None
    }

    pub fn dir_entries(&self) -> Option<Vec<DirEntry>> {
        if self.is_directory() == false {
            return None;
        }

        let mut v: Vec<DirEntry> = Vec::new();
        let mut file_off = 0;
        let mut bdio = BufferedDiskIO::new(self.disk_index).unwrap();
        loop {
            let (memaddr, ioc) = self.read_int(&mut bdio, file_off, size_of::<DirEntry>());
            if let IOCompletion::Successful(_len) = ioc {
                unsafe {
                    let dir_ent = (memaddr as *const DirEntry).read();
                    if dir_ent.name[0] != DirEntry::NAME_0_FREE_ENTRY &&
                            dir_ent.name[0] != DirEntry::NAME_0_LAST_ENTRY &&
                            dir_ent.attr & DirEntry::ATTR_LONG_FILE_NAME == 0{
                        v.push(dir_ent);
                    } else if dir_ent.name[0] == DirEntry::NAME_0_LAST_ENTRY {
                        break;
                    }
                }
                file_off += size_of::<DirEntry>();
            } else {
                // Hit the end of the directory content 
                break;
            }    
        }
        Some(v)
    }

    pub fn capacity(&self) -> usize {
        match &self.au_map {
            AUMap::FAT1216Root {start_sec: _, entry_count}  => {
                ((32 * entry_count + self.sector_size - 1) / 
                                    self.sector_size) as usize
            },
            AUMap::FATFile { cluster_list }                 => {
                cluster_list.len() * self.cluster_size as usize
            }
        }
    }

    pub fn size_bytes(&self) -> usize {
        self.dir_entry.size_bytes()
    }

    pub fn is_directory(&self) -> bool {
        self.dir_entry.attr & DirEntry::ATTR_DIRECTORY > 0
    }

    pub fn first_sector_of_cluster(&self, cluster: u32) -> u64 {
        self.data_start_sector as u64 + 
            (cluster - 2) as u64 * self.cluster_sectors as u64
    }

    fn read_int(&self, bdio: &mut BufferedDiskIO, file_offset: usize,
                                    num_bytes: usize) -> (usize, IOCompletion) {
        if file_offset >= self.capacity() {
            return (0, IOCompletion::OutOfBoundIO);
        }
        // Adjust the length if the request hits EOF
        let len: usize;
        if self.is_directory() == false &&
            file_offset + num_bytes > self.size_bytes() {
            len = self.size_bytes() - file_offset;
        } else {
            len = num_bytes;
        }

        // TODO - handler requests that cross cluster boundary
        let cluster_index = file_offset / self.cluster_size as usize;
        let cluster_offset= file_offset % self.cluster_size as usize;
        if cluster_offset + len > self.cluster_size as usize {
            panic!("fat::File::read doesn't support cross cluster reads!");
        }
        let first_sector_index: u64;
        match &(self.au_map) {
            AUMap::FAT1216Root { start_sec, entry_count: _ }    => {
                first_sector_index = *start_sec as u64;
            },
            AUMap::FATFile { cluster_list }                     => {
                first_sector_index = self.first_sector_of_cluster(
                                                cluster_list[cluster_index]);
            }
        }
         
        let disk_off = (self.first_lba + first_sector_index) * 
                        self.sector_size as u64 + cluster_offset as u64;

        // klog!("File offset: {}, cluster index: {}, cluster offset: {}, #bytes: {}, first_sector_index: {}, disk_off: {}\n",
        //         file_offset, cluster_index, cluster_offset, len,
        //         first_sector_index, disk_off
        // );
        let (mem_addr, ioc) = bdio.read(
                                DiskAddress::ByteAddr { addr: disk_off }, len);
        (mem_addr, ioc)
    }
    pub fn read(&self, file_offset: usize, num_bytes: usize) -> (usize, IOCompletion) {
        let mut bdio = BufferedDiskIO::new(self.disk_index).unwrap();
        self.read_int(&mut bdio, file_offset, num_bytes)
    }
}

impl Drop for FATFile {
    fn drop(&mut self) {
        // klog!("dropping FATFile: {}\n", self.dir_entry.name());
    }
}

impl Debug for FATFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.au_map {
            AUMap::FAT1216Root { start_sec, entry_count }       => {
                write!(f, "{:?}: {} Entries starting at sector {}\n", 
                    self.dir_entry, entry_count, start_sec)
            },
            AUMap::FATFile { cluster_list }                     => {
                write!(f, "{:?} #of clusters: {}\n{:?}", 
                    self.dir_entry, cluster_list.len(), cluster_list)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct FATVolume {
    pub kind:           FATKind,
    pub label:          String,
    first_lba:          u64,    // Physical sector #
    last_lba:           u64,    // Physical sector #
    disk_index:         usize,
    // Information retrieved/derived from the boot sector 
    // All sector numbers are logical, i.e., relative to the start of the volume
    sector_size:        u16,    // Bytes per Sector
    cluster_sectors:    u8,     // Sectors per Cluster
    cluster_size:       u32,    // Bytes per Cluster
    volume_sectors:     u32,    // Total volume size in sectors
    // AREA 1) FAT
    fat_count:          u8,
    fat_start_sector:   u32,    // = rsvd_sector_count
    fat_sectors:        u32,    // = fat[16/32]_size * num_fats;
    // AREA 2) Root Directory (FAT12/16)
    root_start_sector:  u32,    // fat_start_sector + fat_sectors
    root_entry_count:   u16,    // 224, 512, 0 for FAT12, FAT16, FAT32
    root_sectors:       u16,    // (32 * root_entry_count + sector_sz - 1) / sector_sz
    root32_cluster0:    u32,    // First cluster of the root directory for FAT32
    // AREA 3) Data Area
    data_start_sector:  u32,    // root_start_sector + root_sectors
    data_sectors:       u32,    // volume_sectors - data_start_sector
    data_clusters:      u32,    // data_sectors / cluster_sectors
    // FAT type determination based on # of clusters:
    // FAT12:          cluster_count <= 4085
    // FAT16: 4086  <= cluster_count <= 65525
    // FAT32: 65526 <= cluster_count
    // 
}

impl FATVolume {
    // Creates a FATVolume object by reading from a disk partition.
    // Returns None if the partition is not FAT12/16/32.
    // Creates an internal copy of the disk for further IO operations
    pub fn mount(disk_index: usize, part_index: usize,
                start_lba: u64, end_lba: u64) -> bool {
        if !(end_lba > start_lba && end_lba - start_lba > 2) {
            return false;
        }
        let mut bdio = BufferedDiskIO::new(disk_index).unwrap();
        
        // Is it FAT12/16?
        let bs: *const BootSector16;
        let (buff, _) = bdio.read(
                        DiskAddress::LBA { lba: start_lba, block_offset: 0 },
                        size_of::<BootSector16>());
        bs = buff as *const BootSector16;

        
        let mut fatkind = FATKind::Unknown;
        unsafe {
        if str::from_utf8(&(*bs).fs_type) == str::from_utf8(b"FAT12   ") {
            fatkind = FATKind::FAT12;
        } else if str::from_utf8(&(*bs).fs_type) ==str::from_utf8(b"FAT16   ") {
                fatkind = FATKind::FAT16;
        }
        if (*bs).boot_sig == 0x29 && 
            (fatkind == FATKind::FAT12 || fatkind == FATKind::FAT16) {
            let fat_start_sector:   u32 = (*bs).rsvd_sector_count as u32;
            let fat_sectors:        u32 = (*bs).fat16_size as u32 * 
                                            (*bs).num_fats as u32;
            let root_start_sector:  u32 = fat_start_sector + fat_sectors;
            let root_sectors:       u16 = (32 * (*bs).root_ent_count + 
                                            (*bs).bytes_per_sector - 1) / 
                                            (*bs).bytes_per_sector;
            let data_start_sector:  u32 = root_start_sector + 
                                            root_sectors as u32;
            let data_sectors:       u32 = (*bs).fat16_tot_sec as u32 - 
                                            data_start_sector;
            let cluster_count:      u32 = data_sectors / 
                                            (*bs).sectors_per_cluster as u32;
            let vol = Self {
                kind:               fatkind,
                disk_index:         disk_index,
                label:              String::from_utf8
                                        ((*bs).volume_label.to_vec()).unwrap(),
                first_lba:          start_lba,
                last_lba:           end_lba,
                sector_size:        (*bs).bytes_per_sector,
                cluster_sectors:    (*bs).sectors_per_cluster,
                cluster_size:       (*bs).sectors_per_cluster as u32 *
                                    (*bs).bytes_per_sector as u32,
                volume_sectors:     (*bs).fat16_tot_sec as u32,
                fat_count:          (*bs).num_fats,
                fat_start_sector:   fat_start_sector,
                fat_sectors:        fat_sectors,
                root_start_sector:  root_start_sector,
                root_sectors:       root_sectors,
                root_entry_count:   (*bs).root_ent_count,
                root32_cluster0:    0,
                data_start_sector:  data_start_sector,
                data_sectors:       data_sectors,
                data_clusters:      cluster_count
            };
            // Register the detected volume here and with the VFS
            let mnt_name = format!("disk{}.{}", disk_index, part_index);
            let mnt_obj = MountPoint {  name:       mnt_name.clone(),
                                        fops:       fat_fops_handler};
            if MountPoint::mount(mnt_obj) {
                // Mount-point added successfully. Register the Volume here
                FAT_VOLUMES.lock().insert(mnt_name, Arc::new(vol));
                return true;
            } else {
                panic!("Mount-point {} not registered with VFS\n", mnt_name);
            }
        }
        }

        // Is it FAT32?
        let bs: *const BootSector32;
        let (buff, _) = bdio.read(
                        DiskAddress::LBA { lba: start_lba, block_offset: 0 },
                        size_of::<BootSector32>());
        bs = buff as *const BootSector32;
        unsafe {
        if str::from_utf8(&(*bs).fs_type) == str::from_utf8(b"FAT32   ") {
            fatkind = FATKind::FAT32;
        }
        if (*bs).boot_sig == 0x29 && fatkind == FATKind::FAT32 {
            let fat_start_sector:   u32 = (*bs).rsvd_sector_count as u32;
            let fat_sectors:        u32 = (*bs).fat32_sectors as u32 * 
                                            (*bs).num_fats as u32;
            let root_start_sector:  u32 = fat_start_sector + fat_sectors;
            let root_sectors:       u16 = (32 * (*bs).root_ent_count +
                                            (*bs).bytes_per_sector - 1) / 
                                            (*bs).bytes_per_sector;
            let data_start_sector:  u32 = root_start_sector + root_sectors as u32;
            let data_sectors:       u32 = (*bs).fat32_tot_sec - data_start_sector;
            let cluster_count:      u32 = data_sectors / (*bs).sectors_per_cluster as u32;
            let vol = Self {
                kind:               fatkind,
                disk_index:         disk_index,
                label:              String::from_utf8
                                        ((*bs).volume_label.to_vec()).unwrap(),
                first_lba:          start_lba,
                last_lba:           end_lba,
                sector_size:        (*bs).bytes_per_sector,
                cluster_sectors:    (*bs).sectors_per_cluster,
                cluster_size:       (*bs).sectors_per_cluster as u32 *
                                    (*bs).bytes_per_sector as u32,
                volume_sectors:     (*bs).fat32_tot_sec,
                fat_count:          (*bs).num_fats,
                fat_start_sector:   fat_start_sector,
                fat_sectors:        fat_sectors,
                root_start_sector:  root_start_sector,
                root_sectors:       root_sectors,
                root_entry_count:   (*bs).root_ent_count,
                root32_cluster0:    (*bs).root_cluster,
                data_start_sector:  data_start_sector,
                data_sectors:       data_sectors,
                data_clusters:      cluster_count
            };
            // Register the detected volume here and with the VFS
            let mnt_name = format!("disk{}.{}", disk_index, part_index);
            let mnt_obj = MountPoint {  name:       mnt_name.clone(),
                                        fops:       fat_fops_handler };
            if MountPoint::mount(mnt_obj) {
                // Mount-point added successfully. Register the Volume here
                FAT_VOLUMES.lock().insert(mnt_name, Arc::new(vol));
                return true;
            } else {
                panic!("Mount-point {} not registered with VFS\n", mnt_name);
            }
        }
        }
        false
    }

    fn fat_entry_from_value(&self, value: u32) -> FATEntry {
        match self.kind {
            FATKind::FAT12 => match value {
                0x0                     => FATEntry::Free,
                0x1                     => FATEntry::Reserved,
                0x2..=0xFF6             => FATEntry::InUse(value),
                0xFF7                   => FATEntry::BadCluster,
                0xFF8..=0xFFF           => FATEntry::EndOfChain,
                _                       => FATEntry::Invalid
            },
            FATKind::FAT16              => match value {
                0x0                     => FATEntry::Free,
                0x1                     => FATEntry::Reserved,
                0x2..=0xFFF6            => FATEntry::InUse(value),
                0xFFF7                  => FATEntry::BadCluster,
                0xFFF8..=0xFFFF         => FATEntry::EndOfChain,
                _                       => FATEntry::Invalid
            },
            FATKind::FAT32  => match value {
                0x0                     => FATEntry::Free,
                0x1                     => FATEntry::Reserved,
                0x2..=0x0FFFFFF6        => FATEntry::InUse(value),
                0x0FFFFFF7              => FATEntry::BadCluster,
                0x0FFFFFF8..=0x0FFFFFF8 => FATEntry::EndOfChain,
                _                       => FATEntry::Invalid
            },
            FATKind::Unknown => FATEntry::Invalid
        }
    }

    fn load_fat_entry(&self, bdio: &mut BufferedDiskIO, i: usize) -> FATEntry {
        let entry;
        match self.kind {
            FATKind::FAT12  => {
                // FAT area is an array of 12-bit entries
                // The last entry can cross the sector boundary - TODO Test this
                // byte_off
                //          +---------------------+
                //        0 |       FAT[0]    LSB |
                //          |----------+----------|
                //        1 |FAT[1] LSB|MSB FAT[0]|
                //          |----------+----------|
                //        2 |MSB    FAT[1]        |
                //          |---------------------|
                let (addr, _) = bdio.read(DiskAddress::LBA {
                        lba: self.first_lba + self.fat_start_sector as u64 +
                        (i + (i / 2)) as u64 / self.sector_size as u64,
                        block_offset: (i + (i / 2)) as u32 %
                                        self.sector_size as u32
                }, 2);
                if i & 1 == 0 {
                    // Even entry
                    unsafe {
                        let ptr = addr as *const u8;
                        entry = ptr.add(0).read() as u32 |
                                ((ptr.add(1).read() as u32) & 0x0F) << 8;
                    }
                } else {
                    // Odd entry
                    unsafe {
                        let ptr = addr as *const u8;
                        entry = (ptr.add(0).read() as u32) >> 4 |
                                (ptr.add(1).read() as u32) << 4;
                    }
                }
            },
            FATKind::FAT16  => {
                // FAT area is an array of 16-bit entries
                let (addr, _) = bdio.read(DiskAddress::LBA {
                        lba:
                            self.first_lba + self.fat_start_sector as u64 +
                            (i as u64 * 2 / self.sector_size as u64),
                        block_offset:
                            ((i * 2) % self.sector_size as usize) as u32
                }, 2);
                unsafe {
                    entry = (addr as *const u16).read() as u32;
                }
            },
            FATKind::FAT32  => {
                // FAT area is an array of 32-bit entries
                let (addr, _) = bdio.read(DiskAddress::LBA {
                        lba:
                            self.first_lba + self.fat_start_sector as u64 +
                            (i as u64 * 4 / self.sector_size as u64),
                        block_offset:
                            ((i * 4) % self.sector_size as usize) as u32
                }, 4);
                unsafe {
                    entry = (addr as *const u32).read() & 0x0FFFFFFF;
                }
            },
            _   => {
                panic!("load_fat_entry on unknown volume.");
            }
        }
        self.fat_entry_from_value(entry)
    }
}


#[repr(C, packed)]
#[derive(Default)]
struct BootSector16 {
    jmp_instr:          [u8; 3],
    oem_name:           [u8; 8],
    bytes_per_sector:   u16,
    sectors_per_cluster:u8,
    rsvd_sector_count:  u16,// # of sectors in reserved area. 32 for FAT32
    num_fats:           u8, // # of FAT copies. Should always be 2.
    root_ent_count:     u16,// Must be 0 for FAT32
    fat16_tot_sec:      u16,// Must be 0 for FAT32
    media_type:         u8,
    fat16_size:         u16,// Must be 0 for FAT32
    sec_per_track:      u16,
    num_heads:          u16,
    hidden_sectors:     u32,
    fat32_tot_sec:      u32,// Size of the FAT32 partition in sectors
    drv_num:            u8,
    rsvd:               u8,
    boot_sig:           u8, // Must be 0x29
    volume_id:          u32,// Volume Serial Number
    volume_label:       [u8;11],
    fs_type:            [u8;8], // "FAT12   ", "FAT16   " or "FAT     "
    padding:            u16,
}

#[repr(C, packed)]
#[derive(Default)]
struct BootSector32 {
    jmp_instr:          [u8; 3],
    oem_name:           [u8; 8],
    bytes_per_sector:   u16,
    sectors_per_cluster:u8,
    rsvd_sector_count:  u16,// # of sectors in reserved area. 32 for FAT32
    num_fats:           u8, // # of FAT copies. Should always be 2.
    root_ent_count:     u16,// Must be 0 for FAT32
    fat16_tot_sec:      u16,// Must be 0 for FAT32
    media_type:         u8,
    fat16_size:         u16,// Must be 0 for FAT32
    sec_per_track:      u16,
    num_heads:          u16,
    hidden_sectors:     u32,
    fat32_tot_sec:      u32,// Size of the FAT32 partition in sectors
    fat32_sectors:      u32,// Size of a FAT in sectors
    ext_flags:          u16,
    fs_ver:             u16,
    root_cluster:       u32, // ** First cluster number of the root directory.
    fs_info:            u16,
    bk_boot_sector:     u16,
    rsvd0:              [u8; 12],
    drv_num:            u8,
    rsvd1:              u8,
    boot_sig:           u8, // Must be 0x29
    volume_id:          u32,// Volume Serial Number
    volume_label:       [u8; 11],
    fs_type:            [u8; 8],    // "FAT32     "

}

//
// VFS Interface 
//
static FAT_VOLUMES: Spinlock<BTreeMap<String, Arc<FATVolume> >> =
                        Spinlock::new(BTreeMap::new());
static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(0);
static OPEN_FILES:  Spinlock<BTreeMap< usize, Arc<FATFile> > > =
                        Spinlock::new(BTreeMap::new());

fn fat_fops_handler(op: FileOperation) -> IOCompletion {
    match op {
        FileOperation::Open { path }                        => {
            match MountPoint::get_mntname_devpath(path) {
                Some((mnt_name, fpath))  => {
                    let vol;
                    let volumes = FAT_VOLUMES.lock();
                    if let Some(vol_ptr) = volumes.get(mnt_name){
                        vol = vol_ptr.clone();
                        drop(volumes); // unlock
                        fopen(&vol, fpath)
                    } else {
                        // Volume not found
                        IOCompletion::InvalidDrive
                    }
                },
                None                        => {
                    IOCompletion::InvalidPath
                }
            }
        },
        FileOperation::Enum { hnd, out }                  => {
            fenum(hnd, out)
        },
        FileOperation::Read { hnd, off, buff }              => {
            fread(hnd, off, buff)
        },
        FileOperation::Write { hnd, off, buff }             => {
            fwrite(hnd, off, buff)
        }
        FileOperation::Exec { hnd, func, buff }             => {
            fexec(hnd, func, buff)
        }
        FileOperation::Close { hnd }                        => {
            fclose(hnd)
        }
    }
}

fn fopen(vol: &FATVolume, fpath: &str) -> IOCompletion {
    let file_opt = FATFile::from_path(vol, fpath);
    {
        let mut file_list = OPEN_FILES.lock(); // WRITE LOCK
        if let Some(fobj) = file_opt {
            let hnd = NEXT_HANDLE.fetch_add(1, Relaxed);
            file_list.insert(hnd, Arc::new(fobj));
            return IOCompletion::Successful(hnd);
        }
    }
    IOCompletion::InvalidPath
}

fn fenum(hnd: usize, out: &mut Vec<DirectoryEntry>) -> IOCompletion {
    let file;
    {
        let file_list = OPEN_FILES.lock();
        if let Some(fobj) = file_list.get(&hnd) {
            file = fobj.clone();
        } else {
            return IOCompletion::InvalidHandle;
        }
    }    

    if file.is_directory() {
        let dentry_lst = file.dir_entries().unwrap();
        for child in dentry_lst {
            let mut item = DirectoryEntry {
                name: child.name(),
                size: child.size_bytes(),
                flags: 0
            };
            if child.attr & DirEntry::ATTR_DIRECTORY > 0 {
                item.flags |= DirectoryEntry::FLG_DIRECTORY;
            }
            if child.attr & DirEntry::ATTR_ARCHIVE > 0 {
                item.flags |= DirectoryEntry::FLG_ARCHIVE;
            }
            if child.attr & DirEntry::ATTR_HIDDEN > 0 {
                item.flags |= DirectoryEntry::FLG_HIDDEN;
            }
            if child.attr & DirEntry::ATTR_SYSTEM > 0 {
                item.flags |= DirectoryEntry::FLG_SYSTEM;
            }
            if child.attr & DirEntry::ATTR_READ_ONLY > 0 {
                item.flags |= DirectoryEntry::FLG_PERM_READ |
                                DirectoryEntry::FLG_PERM_EXEC;
            } else {
                item.flags |= DirectoryEntry::FLG_PERM_READ |
                                DirectoryEntry::FLG_PERM_WRITE |
                                DirectoryEntry::FLG_PERM_EXEC;
            }
            out.push(item);
        }
    } else {
        // Not a directory
        return IOCompletion::InvalidOp;
    }
    IOCompletion::Successful(out.len())
}

fn fread(hnd: usize, off: usize, buff: &mut [u8]) -> IOCompletion {
    
    let file;
    {
        let mut file_list = OPEN_FILES.lock();
        if let Some(fobj) = file_list.get_mut(&hnd) {
            file = fobj.clone();
        } else {
            return IOCompletion::InvalidHandle;
        }
    }

    if file.is_directory() == false {
        // Normal File Read
        let (memaddr, ioc) = file.read(off, buff.len());
        if let IOCompletion::Successful(len) = ioc {
            let ptr = memaddr as *const u8;
            // dump_memory_ascii(memaddr, 100);
            // Calculate size and create a slice
            unsafe {
                buff[0..len].copy_from_slice(slice::from_raw_parts(ptr, len));
            }
        }
        return ioc;
    }
    IOCompletion::InvalidOp

}

fn fwrite(_hnd: usize, _off: usize, _buff: &[u8]) -> IOCompletion {
    IOCompletion::InvalidOp
}

fn fexec(_hnd: usize, _func: usize, _buff: &mut [u8]) -> IOCompletion {
    // Things like delete, move and copy for the FS driver go here
    IOCompletion::InvalidOp
}

fn fclose(hnd: usize) -> IOCompletion {
    let mut file_list = OPEN_FILES.lock();
    if let Some(_) = file_list.remove(&hnd) {
        return IOCompletion::Successful(0);
    }
    IOCompletion::InvalidHandle
}

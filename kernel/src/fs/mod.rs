//
// BlightOS Kernel
//
// Virtual Filesystem Interface
// 
// Allows drivers to register a mount point and handler file operations
// issued against that mount point.
//
// Absolute addresses are formed as [mount-point-name]:[path], for example:
// keyboard:/0 -> registered by the keyboard driver for the first connected kbd
// disk0.0:/dir1/dir11/file111.txt -> registered by a FS for the first partition
//                                    on the first disk.
//

use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::drivers::storage::{IOCompletion, get_disk};
use crate::util::*;
pub mod gpt;
pub mod fat;


#[derive(Debug)]
pub struct DirectoryEntry {
    pub name:           String,
    pub flags:          usize,
    pub size:           usize, // in bytes
    // pub owner_uid:      u32,
    // pub owner_gid:      u32,
    // pub created:        u64,
    // pub accessed:       u64,
    // pub modified:       u64
}
impl DirectoryEntry {
    pub const FLG_DIRECTORY:    usize = 0x1;
    //pub const FLG_SOFT_LINK:    usize = 0x2;
    //pub const FLG_HARD_LINK:    usize = 0x4;
    pub const FLG_DEVICE:       usize = 0x8;
    pub const FLG_HIDDEN:       usize = 0x10;
    pub const FLG_ARCHIVE:      usize = 0x20;
    pub const FLG_SYSTEM:       usize = 0x40;
    pub const FLG_PERM_READ:    usize = 0x100;
    pub const FLG_PERM_WRITE:   usize = 0x200;
    pub const FLG_PERM_EXEC:    usize = 0x400;
}
///
/// Drivers wishing to register mount points in the VFS must implement the
/// an File Operations Handler that perform the following operations.
/// 
pub enum FileOperation<'a> {
    //
    // The path string includes the mount-point name in case the handler is
    // shared for multiple devices mounted with different prefixes
    // 
    // hnd: Handle is driver-specific and should be used to associate requests
    //      (following an initial open) to the resource associated with the path
    //      This is different from a file-descriptor, which is process-specific.
    //      Each process-address space maintains its own FD->HND mapping!
    //
    Open{path: &'a str}, // IOCompletion::Successful(hnd)
    Enum{hnd: usize, out: &'a mut Vec<DirectoryEntry>},
    Read{hnd: usize, off: usize, buff: &'a mut [u8]},
    Write{hnd: usize, off: usize, buff: &'a [u8]},
    Exec{hnd: usize, func: usize, buff: &'a mut [u8]},
    Close{hnd: usize}
}
type FileOpsHandler = fn(op: FileOperation) -> IOCompletion;

#[derive(Clone)]
pub struct MountPoint {
    // File operations addressing mountpoint:/the/address will be handed by the
    // FileOpsHandler of the mount point whose name is mountpoint
    pub name:       String,
    pub fops:       FileOpsHandler,
}

// Todo replace spinlock with RWLock and use Arc for ref-counting of each
// Mount point
static MOUNT_POINTS: Spinlock<BTreeMap<String, Arc<MountPoint> >> =
                        Spinlock::new(BTreeMap::new());

//
// Mount point is the entry point to all FS operations:
// mount, unmount, lsmnt, fopen, fread, fwrite, fctrl, fclose, dlist
//
// Every path sent to this struct should be in the following format:
// mount-point-name:/absolute/path/to/the/file/or/director
// e.g., disk0.0:boot/efi/bootx64.efi
// 
impl MountPoint {
    // Constructors
    //
    // Returns a copy of the mount-point object whose name is included in the
    // path, and increases the reference counter of the mount point
    pub fn from_path(path: &str) -> Option<Arc<Self>> {
        if path.is_empty() {
            return None;
        }
        // Find the moun-point name 
        let mnt_name;
        if let Some(collon) = path.find(":") {
            // mount-point-name:/the/rest/of/the/path
            mnt_name = &path[..collon];
        } else {
            // mount-point-name
            mnt_name = &path[..];
        }
        // Retrieve a copy (TODO an Arc pointer) of the mount-point object
        let mut mnt_map = MOUNT_POINTS.lock(); // WRITE LOCK
        if let Some(mnt_point) = mnt_map.get_mut(mnt_name) {
            return Some(mnt_point.clone());
        }
        None
    }

    // Given a full path (e.g., mnt_name:/dir1/dir11), it eliminates the mount
    // point name and returns the path that can be passed to the driver in
    // in charge of the mount-point file operations
    pub fn device_relative_path<'a>(path: &'a str) -> &'a str {
        if let Some(collon) = path.find(":") {
            return &path[(collon+1)..];
        } 
        &path[..]
    }

    pub fn mount_name_in_path<'a>(path: &'a str) -> &'a str {
        if let Some(collon) = path.find(":") {
            return &path[0..collon];
        } 
        &path[..]
    }

    pub fn get_mntname_devpath<'a>(path: &'a str) -> Option<(&'a str, &'a str)> {
        if let Some(collon) = path.find(":") {
            return Some((&path[0..collon], &path[(collon+1)..]));
        }
        None
    }

    //
    // Member helper methods
    // To be used by the system call interface and the parts that issue the
    // fops requests (Not the handlers)
    //

    pub fn fopen(&self, path: &str) -> IOCompletion {
        (self.fops)(FileOperation::Open { path: path })
    }

    pub fn fenum(&self, hnd: usize, out: &mut Vec<DirectoryEntry>) -> IOCompletion {
        (self.fops)(FileOperation::Enum { hnd, out })
    }

    pub fn fread(&self, hnd: usize, off: usize, buff: &mut [u8]) -> IOCompletion {
        (self.fops)(FileOperation::Read { hnd, off, buff })
    }

    pub fn fwrite(&self, hnd: usize, off: usize, buff: &[u8]) -> IOCompletion {
        (self.fops)(FileOperation::Write { hnd, off, buff })
    }

    pub fn fexec(&self, hnd: usize, func: usize, buff: & mut [u8]) -> IOCompletion {
        (self.fops)(FileOperation::Exec { hnd, func, buff })
    }

    pub fn fclose(&self, hnd: usize) -> IOCompletion {
        (self.fops)(FileOperation::Close { hnd })
    }

    //
    // Non-member functions used by the drivers/syscall interface to modify
    // the list of mount-points
    //
    pub fn mount(mount_point: MountPoint) -> bool {
        let mut mps = MOUNT_POINTS.lock(); //WRITE LOCK
        if let Some(_) = mps.get(&mount_point.name) {
            return false; // A mount point with the same name exists
        }
        mps.insert(String::from(mount_point.name.clone()), 
                    Arc::new(mount_point));
        true
    }

    pub fn unmount(mount_name: &str) {
        let mut mps = MOUNT_POINTS.lock(); // WRITE LOCK
        mps.remove(mount_name);
    }

    pub fn list_names() -> Vec<String> {
        let mut res: Vec<String> = Vec::new(); // READ LOCK
        let mps = MOUNT_POINTS.lock();
        for name in mps.keys() {
            res.push(name.clone());
        }
        res
    }

}
impl Drop for MountPoint {
    fn drop(&mut self) {
        klog!("Dropping MountPoint {}\n", self.name);
    }
}

//
// The kernel's initialization routine calls this to mount all supported
// filesystems on every detected storage device
//
pub fn enumerate_filesystems(disk_index: usize) {
    let num_partitions;
    // Check if the disk uses GPT
    num_partitions = gpt::enumerate_partitions(disk_index);
    if num_partitions > 0 {
        // Go over the list of partitions and call the filesystem drivers
    } else {
        let disk = get_disk(disk_index).expect("Disk not found!");
        klog!("  Disk {:?}{}.{} doesn't follow any supported format!\n",
                disk.bus, disk.bus_id, disk.drive_id);
    }
}


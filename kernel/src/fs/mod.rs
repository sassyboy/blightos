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
use crate::Error;
use crate::drivers::storage::get_disk;
use crate::util::*;
pub mod gpt;
pub mod fat;

/// Encapsulates the information about an open file/directory/device.
/// Provides the minimal set of information a process needs to know about a file
/// to perform further operations on it (e.g., read/write/close).
/// The file will be *closed* when the last File instance is dropped.
pub struct File {
    pub mnt:        Arc<MountPoint>,// Mount-point via which the file was opened
    pub dev_hnd:    usize,          // Device/FS-specific handle
    pub mode:       usize,          // Open mode (e.g., read/write/exec)
    pub dir_entry:  DirectoryEntry, // Directory entry of the file
    read_off:       usize,          // Current read offset in the file
    write_off:      usize,          // Current write offset in the file
    ref_count: Arc<Spinlock<usize>>,// Number of File instances sharing the same
                                    // file (for cloning)
}
pub enum FileSeekOrigin {
    Start,
    Current,
    End
}
pub enum FileSeekCursor {
    Read,
    Write
}

impl File {
    pub const MODE_CREATE:  usize = 0x1;
    pub const MODE_READ:    usize = 0x2;
    pub const MODE_WRITE:   usize = 0x4;
    pub const MODE_EXEC:    usize = 0x8;
    pub const MODE_APPEND:  usize = 0x10;

    /// Opens the file/directory/device specified by the full path with the
    /// access mode specified by the mode argument.
    /// open() leads to an open call to the underlying driver managing the file.
    /// Unlike clone(), open() creates a new instance the file.
    pub fn open(full_path: &str, mode: usize) -> Result<File, Error> {
        let mnt = MountPoint::from_path(full_path)?;
        let mut fobj = File {
            mnt:        mnt.clone(),
            dev_hnd:    0,
            mode,
            dir_entry:  DirectoryEntry::new(),
            read_off:   0,
            write_off:  0,
            ref_count:  Arc::new(Spinlock::new(1))
        };
        let dev_hnd = (mnt.fops)(FileOperation::Open{
                                                full_path,
                                                mode,
                                                dent: &mut fobj.dir_entry})?;
        fobj.dev_hnd = dev_hnd;
        Ok(fobj)
    }

    /// Returns a list of entries in the directory.
    /// 
    /// Only valid for directories.
    pub fn enumerate(&self) -> Result<Vec<DirectoryEntry>, Error> {
        if self.dir_entry.flags & DirectoryEntry::FLG_DIRECTORY == 0 {
            return Err(error!(ErrorCode::InvalidOp));
        }
        let mut out: Vec<DirectoryEntry> = Vec::new();
        let _count =(self.mnt.fops)(FileOperation::Enum {
                                                    hnd: self.dev_hnd,
                                                    out: &mut out })?;
        Ok(out)
    }

    /// Reads buff.len() from fd.read_off and updates the read offset by the
    /// number of bytes read.
    /// 
    /// Only valid for readable files opened with read access.
    pub fn read(&mut self, buff: &mut [u8]) -> Result<usize, Error> {
        if self.dir_entry.flags & DirectoryEntry::FLG_PERM_READ == 0 ||
            self.mode & Self::MODE_READ == 0 {
            return Err(error!(ErrorCode::NotAllowed));
        }
        let bytes = (self.mnt.fops)(FileOperation::Read {
                                                hnd: self.dev_hnd,
                                                off: self.read_off,
                                                buff })?;
        self.read_off += bytes;
        Ok(bytes)
    }

    /// Writes buff.len() from fd.write_off and updates the write offset by the
    /// number of bytes written
    /// If the final write offset exceeds the file size, the size is updated to
    /// accordingly assuming that the underlying driver extended the file
    /// to accommodate the write.
    /// 
    /// Only valid for writable files opened with write access.
    pub fn write(&mut self, buff: &[u8]) -> Result<usize, Error> {
        if self.dir_entry.flags & DirectoryEntry::FLG_PERM_WRITE == 0 ||
            self.mode & Self::MODE_WRITE == 0 {
            return Err(error!(ErrorCode::NotAllowed));
        }
        let bytes = (self.mnt.fops)(FileOperation::Write {
                                                hnd: self.dev_hnd,
                                                off: self.write_off,
                                                buff })?;
        self.write_off += bytes;
        if self.write_off > self.dir_entry.size {
            self.dir_entry.size = self.write_off;
        }
        Ok(bytes)
    }

    pub fn get_read_cursor(&self) -> usize {
        self.read_off
    }

    pub fn get_write_cursor(&self) -> usize {
        self.write_off
    }

    pub fn size(&self) -> usize {
        // TODO should consult the underlying driver as the actual size may
        // once file sharing is allowed and multiple processes can have the same
        // file open with different
        self.dir_entry.size
    }

    /// This should be only used by kernel tasks.
    /// The user-space interface maintains its own cursors.
    /// Returns the new offset after seeking.
    pub fn seek(&mut self, delta: isize, origin: FileSeekOrigin,
                            cursor: FileSeekCursor) -> Result<usize, Error> {
        match origin {
            FileSeekOrigin::Start => {
                if delta < 0 || (delta as usize) >= self.size() {
                    // Cannot seek to a negative offset or beyond the end of the
                    // file regardless of read/write
                    klog!("Invalid seek offset: {}, file size: {}\n", delta, self.size());
                    return Err(error!(ErrorCode::InvalidArgument));
                }
                match cursor {
                    FileSeekCursor::Read => {
                        self.read_off = delta as usize;
                        return Ok(self.read_off);
                    },
                    FileSeekCursor::Write => {
                        self.write_off = delta as usize;
                        return Ok(self.write_off);
                    }
                }
            },
            FileSeekOrigin::Current => {
                if delta < 0 && (-delta as usize) > self.read_off ||
                    delta > 0 && (self.read_off + (delta as usize)) > self.size() {
                    return Err(error!(ErrorCode::InvalidArgument));
                }
                match cursor {
                    FileSeekCursor::Read => {
                        self.read_off = ((self.read_off as isize) + delta) as usize;
                        return Ok(self.read_off);
                    },
                    FileSeekCursor::Write => {
                        self.write_off = ((self.write_off as isize) + delta) as usize;
                        return Ok(self.write_off);
                    }
                }
            },
            FileSeekOrigin::End => {
                if delta > 0 || (-delta as usize) > self.size() {
                    return Err(error!(ErrorCode::InvalidArgument));
                }
                match cursor {
                    FileSeekCursor::Read => { 
                        self.read_off = self.size() - (-delta as usize);
                        return Ok(self.read_off);
                    },
                    FileSeekCursor::Write => {
                        self.write_off = self.size() - (-delta as usize);
                        return Ok(self.write_off);
                    }
                }
            }
        }
    }
    /// Executes the custom function number `func` on the device file
    /// 
    /// Only valid for executable device files opened with exec access.
    pub fn execute(&mut self, func: usize, buff: &mut [u8])
                                                    -> Result<usize, Error> {
        if self.dir_entry.flags & DirectoryEntry::FLG_DEVICE == 0 ||
            self.dir_entry.flags & DirectoryEntry::FLG_PERM_EXEC == 0 ||
            self.mode & Self::MODE_EXEC == 0 {
            return Err(error!(ErrorCode::NotAllowed));
        }
        let ret = (self.mnt.fops)(FileOperation::Exec {
                                                hnd: self.dev_hnd,
                                                func,
                                                buff })?;
        Ok(ret)
    }

    fn close(&mut self) -> Result<(), Error> {
        let _ret = (self.mnt.fops)(FileOperation::Close { hnd: self.dev_hnd })?;
        Ok(())
    }
}
impl Drop for File {
    // Closes the file by calling the underlying driver's close operation if
    // this is the last instance of the file (ref_count == 1).
    fn drop(&mut self) {
        {
            let mut count = self.ref_count.lock();
            if *count > 1 {
                *count -= 1;
                return;
            }
        }
        // klog!("Dropping file {} with handle {} on mount-point {}\n",
        //         self.dir_entry.name, self.dev_hnd, self.mnt.name);
        self.close().ok();
    }
}
impl Clone for File {
    /// Creates a new instance of the file that shares the same underlying
    /// resource, i.e., it doesn't call the underlying driver's open operation.
    fn clone(&self) -> Self {
        let mut count = self.ref_count.lock();
        *count += 1;
        File {
            mnt: self.mnt.clone(),
            dev_hnd: self.dev_hnd,
            mode: self.mode,
            dir_entry: DirectoryEntry {
                name: self.dir_entry.name.clone(),
                flags: self.dir_entry.flags,
                size: self.dir_entry.size
            },
            read_off: self.read_off,
            write_off: self.write_off,
            ref_count: self.ref_count.clone()
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub name:           String, // File/Directory name (not full path)
    pub flags:          usize,
    pub size:           usize,  // in bytes
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
    pub const FLG_PERM_EXEC:    usize = 0x400; // Can perform exec call

    // Convenience flag combinations for device drivers
    pub const DEV_R_DIR_FLAGS:  usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_DIRECTORY |
                                        Self::FLG_PERM_READ;
    pub const DEV_RX_DIR_FLAGS: usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_DIRECTORY |
                                        Self::FLG_PERM_READ |
                                        Self::FLG_PERM_EXEC;
    pub const DEV_RW_DIR_FLAGS: usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_DIRECTORY |
                                        Self::FLG_PERM_READ |
                                        Self::FLG_PERM_WRITE;
    pub const DEV_RWX_DIR_FLAGS:usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_DIRECTORY |
                                        Self::FLG_PERM_READ |
                                        Self::FLG_PERM_WRITE |
                                        Self::FLG_PERM_EXEC;
    pub const DEV_R_FILE_FLAGS: usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_PERM_READ;
    pub const DEV_RX_FILE_FLAGS:usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_PERM_READ |
                                        Self::FLG_PERM_EXEC;
    pub const DEV_W_FILE_FLAGS:usize  = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_PERM_WRITE;
    pub const DEV_WX_FILE_FLAGS:usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_PERM_WRITE |
                                        Self::FLG_PERM_EXEC;
    pub const DEV_RW_FILE_FLAGS:usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_PERM_READ |
                                        Self::FLG_PERM_WRITE;
    pub const DEV_RWX_FILE_FLAGS:usize = Self::FLG_SYSTEM |
                                        Self::FLG_DEVICE |
                                        Self::FLG_PERM_READ |
                                        Self::FLG_PERM_WRITE |
                                        Self::FLG_PERM_EXEC;

    pub const fn new() -> Self {
        DirectoryEntry {
            name: String::new(),
            flags: 0,
            size: 0
        }
    }
}

///
/// Drivers wishing to register mount points in the VFS must implement the
/// a File Operations Handler (FileOpsHandler) and call MountPoint::mount().
/// 
/// FileOpsHandler is a function that takes a FileOperation as an argument and
/// performs the various Operations defined in the FileOperation enum.
/// 
/// The operations Open, Entry and Close are mandatory for proper VFS listing in
/// the terminal/file-manager for regular/device files. 
/// 
/// In addition to that, Enum is mandatory for regular/device directories.
/// 
/// NOTE: The argument Handle (hnd) in FileOperation members is driver-specific
///       and should be used to associate requests (following an initial open)
///       to the resource in question.
///       This is different from a file-descriptor, which is process-specific.
///       Each process-address space maintains its own FD->HND mapping, and the
///       system call handlers in kernel.rs perform the mapping and call these
///       operations with the driver handle (hnd) as an argument.
/// 
pub enum FileOperation<'a> {

    /// Attempts to open the file/directory/device specified by the path with
    /// the access mode specified by the mode argument.
    /// 
    /// The `path` string is always a full path, which includes the mount-point
    /// name followed by a colon and the absolute path to the
    /// file/directory/device to be opened.
    /// E.g. of a full path: disk0.0:/dir1/dir11/file111.txt
    ///                      audio:/output/vol
    /// The driver should parse the path and perform the open operation on the
    /// resource specified by the path. In case the driver is only responsible
    /// for one mount-point, it can eliminate the mount-point name using
    /// MountPoint::device_relative_path() from the full path.
    /// The driver should associate the opened resource with a handle
    /// (dev_hnd: usize) that it maintains internally, and refer to that for any
    /// subsequent operations on the same resource (e.g., read/write/close).
    /// 
    /// The `mode` argument is a bitmask that specifies the access mode for
    /// opening. The bit values are defined in FileHandle::MODE_* constants.
    /// 
    /// The `dent` argument is a mutable reference to a DirectoryEntry struct
    /// that should be used by the driver to return the directory entry
    /// information of the opened resource.
    /// 
    /// Returns Ok(dev_hnd) if successful.
    /// The MountPoint will use the tuple to form a FileHandle object to the
    /// caller.
    Open{full_path: &'a str, mode: usize, dent: &'a mut DirectoryEntry},
    
    /// Enum is only valid for directories (or device entries with children),
    /// and should return a list of entries in the directory in the buffer
    /// provided by the caller in `out`.
    /// 
    /// Returns Ok(count) if successful
    Enum{hnd: usize, out: &'a mut Vec<DirectoryEntry>},

    /// Reads from the file/device associated with the handle `hnd` into the
    /// buffer provided by the caller in `buff`, starting from the offset `off`
    /// in the file/device.
    /// The driver should read at most `buff.len()` bytes into the buffer and
    /// return the number of bytes read in the IOCompletion result.
    /// 
    /// Returns Ok(bytes_read) if successful
    Read{hnd: usize, off: usize, buff: &'a mut [u8]},

    /// Writes to the file/device associated with the handle `hnd` from the
    /// buffer provided by the caller in `buff`, starting from the offset `off`
    /// in the file/device.
    /// 
    /// The driver should write at most `buff.len()` bytes from the buffer and
    /// return the number of bytes written in the IOCompletion result.
    /// 
    /// Returns Ok(bytes_written) if successful
    Write{hnd: usize, off: usize, buff: &'a [u8]},

    /// Executes the custom (resource-dependent) function number `func` on the
    /// device resource associated with the handle `hnd`. The buffer provided
    /// by the caller in `buff` can be used to pass arguments to the function
    /// and/or return results from the function, depending on the specific
    /// function.
    /// 
    /// Returns Ok(result) if successful, where `result`
    /// is a function-specific value returned by the executed function.
    Exec{hnd: usize, func: usize, buff: &'a mut [u8]},

    /// Closes the file/directory/device associated with the handle `hnd` and
    /// releases any resources associated with the handle in the driver.
    /// Returns Ok(0) if successful
    Close{hnd: usize}
}
type FileOpsHandler = fn(op: FileOperation) -> Result<usize, Error>;

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
    pub fn from_path(path: &str) -> Result<Arc<Self>, Error> {
        if path.is_empty() {
            return Err(error!(ErrorCode::InvalidPath));
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
            return Ok(mnt_point.clone());
        }
        Err(error!(ErrorCode::InvalidMountPoint))
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


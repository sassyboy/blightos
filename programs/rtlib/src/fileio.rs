///
/// Filesystem
///

use crate::{Exception, ErrorCode};
use crate::env::current_dir;
use crate::syscall::*;
use alloc::string::String;
use crate::*;

pub struct File {
    fd:         usize,
    open:       bool,
    flags:      usize,
    size:       usize,
    rd_offset:  usize,
    wr_offset:  usize
}
impl File {
    const FLG_DIRECTORY:    usize = 0x1;
    //pub const FLG_SOFT_LINK:    usize = 0x2;
    //pub const FLG_HARD_LINK:    usize = 0x4;
    const FLG_DEVICE:       usize = 0x8;
    const FLG_HIDDEN:       usize = 0x10;
    const FLG_ARCHIVE:      usize = 0x20;
    const FLG_SYSTEM:       usize = 0x40;
    const FLG_PERM_READ:    usize = 0x100;
    const FLG_PERM_WRITE:   usize = 0x200;
    const FLG_PERM_EXEC:    usize = 0x400;

    pub const MODE_CREATE:  usize = 0x1;
    pub const MODE_READ:    usize = 0x2;
    pub const MODE_WRITE:   usize = 0x4;
    pub const MODE_EXEC:    usize = 0x8;
    pub const MODE_APPEND:  usize = 0x10;
    pub const MODE_RX:      usize = Self::MODE_READ | Self::MODE_EXEC;
    pub const MODE_RW:      usize = Self::MODE_READ | Self::MODE_WRITE;
    pub const MODE_RWX:     usize = Self::MODE_READ | Self::MODE_WRITE |
                                                    Self::MODE_EXEC;

    pub const fn new() -> Self {
        Self {
            fd:         0,
            open:       false,
            flags:      0,
            size:       0,
            rd_offset:  0,
            wr_offset:  0
        }
    }

    pub fn from_path(path: &Path, mode: usize) -> Result<Self, Exception> {
        let fobj = fopen(path.as_str(), mode)?;
        Ok(Self {
            fd:         fobj.fd,
            open:       true,
            flags:      fobj.attr,
            size:       fobj.size,
            rd_offset:  0,
            wr_offset:  0
        })
    }

    pub fn open(&mut self, path: &Path, mode: usize) -> Result<(), Exception> {
        let fobj = fopen(path.as_str(), mode)?;
        self.fd         = fobj.fd;
        self.open       = true;
        self.flags      = fobj.attr;
        self.size       = fobj.size;
        self.rd_offset  = 0;
        self.wr_offset  = 0;
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_dir(&self) -> bool {
        self.flags & Self::FLG_DIRECTORY != 0
    }

    pub fn is_device(&self) -> bool {
        self.flags & Self::FLG_DEVICE != 0
    }

    pub fn is_hidden(&self) -> bool {
        self.flags & Self::FLG_HIDDEN != 0
    }

    pub fn is_archive(&self) -> bool {
        self.flags & Self::FLG_ARCHIVE != 0
    }

    pub fn is_system(&self) -> bool {
        self.flags & Self::FLG_SYSTEM != 0
    }

    pub fn can_read(&self) -> bool {
        self.flags & Self::FLG_PERM_READ != 0
    }

    pub fn can_write(&self) -> bool {
        self.flags & Self::FLG_PERM_WRITE != 0
    }

    pub fn can_execute(&self) -> bool {
        self.flags & Self::FLG_PERM_EXEC != 0
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Exception> {
        if !self.open {
            return Err(Exception::new(ErrorCode::NotAllowed, "File not open"));
        }
        // Read as much as possible up to the end of the file
        // Trying to read more than the end of the file will cause a seek error,
        // which will be propagated to the caller.
        let mut total: usize = 0;
        let to_read = buffer.len().min(self.size - self.rd_offset);
        // Read in a loop until the requested number of bytes is read
        while total < to_read {
            let n = fread(self.fd, self.rd_offset, &mut buffer[total..])?;
            if n == 0 {
                break;
            }
            total += n;
            self.rd_offset += n;
        }
        Ok(total)
    }

    pub fn write(&mut self, buffer: &[u8]) -> Result<usize, Exception> {
        if !self.open {
            return Err(Exception::new(ErrorCode::NotAllowed, "File not open"));
        }
        let mut total: usize = 0;
        while total < buffer.len() {
            let n = fwrite(self.fd, self.wr_offset, &buffer[total..])?;
            if n == 0 {
                break;
            }
            total += n;
            self.wr_offset += n;
        }
        Ok(total)
    }

    pub fn cursor(&self, cursor: FileSeekCursor) -> Result<usize, Exception> {
        if !self.open {
            return Err(Exception::new(ErrorCode::NotAllowed, "File not open"));
        }
        match cursor {
            FileSeekCursor::Read => {
                if self.can_read() {
                    return Ok(self.rd_offset);
                } else {
                    return Err(Exception::new(ErrorCode::NotAllowed,
                                                        "File not readable"));
                }
            },
            FileSeekCursor::Write => {
                if self.can_write() {
                    return Ok(self.wr_offset);
                } else {
                    return Err(Exception::new(ErrorCode::NotAllowed,
                                                        "File not writable"));
                }
            }
        }
    }

    pub fn seek(&mut self, cursor: FileSeekCursor, origin: FileSeekOrigin,
                                offset: isize) -> Result<usize, Exception> {
        if !self.open {
            return Err(Exception::new(ErrorCode::NotAllowed, "File not open"));
        }
        let pos: usize; // Absolute position to seek to
        let rd = match cursor {
            FileSeekCursor::Read => true,
            FileSeekCursor::Write => false
        };
        let cur_pos = self.cursor(cursor)?;
        match origin {
            FileSeekOrigin::Start => {
                if offset < 0 {
                    return Err(Exception::new(ErrorCode::InvalidArgument,
                                        "Cannot seek to negative position"));
                }
                pos = offset as usize;
            },
            FileSeekOrigin::Current => {
                
                if offset < 0 {
                    if (-offset as usize) > cur_pos {
                        return Err(Exception::new(ErrorCode::InvalidArgument,
                                        "Cannot seek to negative position"));
                    }
                    pos = cur_pos - (-offset as usize);
                } else {
                    pos = cur_pos + (offset as usize);
                }
            },
            FileSeekOrigin::End => {
                if offset > 0 {
                    return Err(Exception::new(ErrorCode::NotSupported,
                                        "File expansion not supported yet."));
                }
                let file_size = self.size();
                if (-offset as usize) > file_size {
                    return Err(Exception::new(ErrorCode::InvalidArgument,
                                        "Cannot seek to negative position"));
                }
                pos = file_size - (-offset as usize);
            }
        };
        if rd {
            self.rd_offset = pos;
        } else {
            self.wr_offset = pos;
        };
        Ok(pos)
    }

    pub fn exec(&self, func: usize, buffer: &mut [u8]) -> Result<usize, Exception> {
        if !self.open {
            return Err(Exception::new(ErrorCode::NotAllowed, "File not open"));
        }
        fexec(self.fd, func, buffer)
    }

    pub fn enum_dir(&self, buffer: &mut [u8]) -> Result<usize, Exception> {
        if !self.open {
            return Err(Exception::new(ErrorCode::NotAllowed, "File not open"));
        }
        fenum(self.fd, buffer)
    }
}
impl Drop for File {
    fn drop(&mut self) {
        if self.open {
            fclose(self.fd);
        }
    }
}

pub enum FileSeekOrigin {
    Start   = 0,
    Current = 1,
    End     = 2
}
pub enum FileSeekCursor {
    Read    = 0,
    Write   = 1,
}

///
/// Represents a full path.
/// The given path can be one of the following:
///   - Absolute path including the mount-point name, e.g. "disk1:/dir/file.ext"
///   - Absolute path from the root of the mount-point (starts with "/")
///   - Relative path from the current directory
/// 
pub struct Path {
    inner: String,
}

impl Path {
    pub fn from(path: &str) -> Self {
        let mut p = Self { inner: String::new() };
        p.make_full_path(path);
        p
    }

    pub fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    //
    // Helper methods
    //
    fn make_full_path(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        let Ok(cur_dir) = current_dir() else {
            return;
        };
        self.inner.clear();
        if path.starts_with("/") {
            // Full address from the start of the mount point
            if let Some(collon) = cur_dir.find(":") {
                self.inner.push_str(&cur_dir[..collon + 1]);
                self.inner.push_str(path);
            } else {
                self.inner.push_str(path);
            }
        } else if let Some(_) = path.find(":") {
            // Absolute address (includes the mount-point name)
            self.inner.push_str(path);
            if path.ends_with(":") { // Mount-point-only path
                self.inner.push_str("/");
            }
        } else {
            // Address relative to the current directory
            self.inner.push_str(cur_dir.as_str());
            if !cur_dir.ends_with("/") {
                self.inner.push('/');
            }
            self.inner.push_str(path);
        }
    }
}

pub fn fopen(path: &str, mode: usize) -> Result<VfsOpenArgs, Exception> {
    let mut ret_val: usize = ErrorCode::Other as usize;
    if path.is_empty() {
        ret_val = ErrorCode::InvalidPath as usize;
    } else {
        let mut args = VfsOpenArgs {
            path_ptr: &(path.as_bytes()[0]) as *const u8 as usize,
            path_len: path.len(),
            mode: mode,
            fd: 0,
            attr: 0,
            size: 0
        };
        syscall(Syscall::Open {
            args_ptr: &mut args as *mut VfsOpenArgs as usize,
            args_len: core::mem::size_of::<VfsOpenArgs>(),
            arg3: 0,
            ret_ptr: &mut ret_val as *mut usize as usize
        });
        if ret_val == ErrorCode::NoError as usize {
            return Ok(args);
        }
    }
    Err(Exception::new(ErrorCode::from(ret_val), "Failed to open file"))
}

pub fn fread(fd: usize, offset: usize, buffer: &mut [u8]) ->
                                                    Result<usize, Exception> {
    let mut ret: usize = 0;
    let mut args = VfsReadWriteArgs {
        fd: fd,
        offset: offset,
        buf_ptr: &mut buffer[0] as *mut u8 as usize,
        buf_len: buffer.len(),
        bytes: 0
    };
    syscall(Syscall::Read {
            args_ptr: &mut args as *mut VfsReadWriteArgs as usize,
            args_len: core::mem::size_of::<VfsReadWriteArgs>(),
            arg3: 0,
            ret_ptr: &mut ret as *mut usize as usize
    });
    if ret == ErrorCode::NoError as usize {
        Ok(args.bytes)
    } else {
        Err(Exception::new(ErrorCode::from(ret), "Failed to read from file"))
    }
}

pub fn fwrite(fd: usize, offset: usize, buffer: &[u8]) -> 
                                                    Result<usize, Exception> {
    let mut ret: usize = 0;
    let mut args = VfsReadWriteArgs {
        fd: fd,
        offset: offset,
        buf_ptr: &buffer[0] as *const u8 as usize,
        buf_len: buffer.len(),
        bytes: 0
    };
    syscall(Syscall::Write {
            args_ptr: &mut args as *mut VfsReadWriteArgs as usize,
            args_len: core::mem::size_of::<VfsReadWriteArgs>(),
            arg3: 0,
            ret_ptr: &mut ret as *mut usize as usize
    });
    if ret == ErrorCode::NoError as usize {
        Ok(args.bytes)
    } else {
        Err(Exception::new(ErrorCode::from(ret), "Failed to write to file"))
    }
}

pub fn fenum(fd: usize, buffer: &mut [u8]) -> Result<usize, Exception> {
    let mut ret: usize = 0;
    let mut args = VfsEnumArgs {
        fd: fd,
        skip: 0, // Todo
        buf_ptr: &mut buffer[0] as *mut u8 as usize,
        buf_len: buffer.len(),
        count: 0
    };
    syscall(Syscall::Enum {
            args_ptr: &mut args as *mut VfsEnumArgs as usize,
            args_len: core::mem::size_of::<VfsEnumArgs>(),
            arg3: 0,
            ret_ptr: &mut ret as *mut usize as usize
    });
    if ret == ErrorCode::NoError as usize {
        Ok(args.count)
    } else {
        Err(Exception::new(ErrorCode::from(ret), "Failed to enumerate directory"))
    }
}

pub fn fexec(fd: usize, func: usize, buffer: &mut [u8]) -> Result<usize, Exception> {
    let mut ret: usize = 0;
    let mut args = VfsExecArgs {
        fd: fd,
        func_code: func,
        args_ptr: &mut buffer[0] as *mut u8 as usize,
        args_len: buffer.len(),
        ret_val: 0
    };
    syscall(Syscall::Exec {
            args_ptr: &mut args as *mut VfsExecArgs as usize,
            args_len: core::mem::size_of::<VfsExecArgs>(),
            arg3: 0,
            ret_ptr: &mut ret as *mut usize as usize
    });
    if ret == ErrorCode::NoError as usize {
        Ok(args.ret_val)
    } else {
        Err(Exception::new(ErrorCode::from(ret), "Failed to execute function"))
    }
}

pub fn fclose(fd: usize) {
    syscall(Syscall::Close { fd });
}

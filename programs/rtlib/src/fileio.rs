///
/// Filesystem
///

use crate::{Exception, ErrorCode};
use crate::env::current_dir;
use crate::syscall::*;
use alloc::string::String;
use crate::*;

pub struct File {
    fd: usize,
    open: bool
}
impl File {
    pub const fn new() -> Self {
        Self {
            fd: 0,
            open: false
        }
    }

    pub fn from_path(path: &Path) -> Result<Self, Exception> {
        if let Some(fd) = fopen(path.as_str()) {
            Ok(Self { fd, open: true })
        } else {
            Err(Exception::new(ErrorCode::NotFound, "Failed to open file"))
        }
    }

    pub fn open(&mut self, path: &Path) -> Result<(), Exception> {
        if let Some(fd) = fopen(path.as_str()) {
            self.fd = fd;
            self.open = true;
            Ok(())
        } else {
            Err(Exception::new(ErrorCode::NotFound, "Failed to open file"))
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn read(&self, buffer: &mut [u8]) -> usize {
        if !self.open {
            return 0;
        }
        let mut total: usize = 0;
        while total < buffer.len() {
            let n = fread(self.fd, &mut buffer[total..]);
            if n == 0 {
            break;
            }
            total += n;
        }
        total
    }

    pub fn seek(&self, cursor: FileSeekCursor, origin: FileSeekOrigin, offset: isize) -> usize {
        if !self.open {
            return 0;
        }
        fseek(self.fd, cursor, origin, offset)
    }

    pub fn write(&self, buffer: &[u8]) -> usize {
        if !self.open {
            return 0;
        }
        let mut total: usize = 0;
        while total < buffer.len() {
            let n = fwrite(self.fd, &buffer[total..]);
            if n == 0 {
                break;
            }
            total += n;
        }
        total
    }

    pub fn exec(&self, func: usize, buffer: &mut [u8]) -> usize {
        if !self.open {
            return 0;
        }
        fexec(self.fd, func, buffer)
    }

    pub fn enum_dir(&self, buffer: &mut [u8]) -> usize {
        if !self.open {
            return 0;
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
    Both    = 2
}

pub struct Path {
    inner: String
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

    pub fn exists(&self) -> bool {
        match fopen(&self.inner) {
            None        => {
                false
            },
            Some(fd)   => {
                fclose(fd);
                true
            }
        }
    }

    pub fn is_dir(&self) -> bool {
        if !self.inner.is_empty() && self.inner.ends_with('/') {
            return true;
        }
        false
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
        let is_dir = self.is_dir();
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
        if is_dir && !self.inner.ends_with("/") {
            self.inner.push_str("/");
        }
    }
}

pub fn fopen(path: &str) -> Option<usize> {
    if !path.is_empty() {
        let mut ret_fd: usize = 0;
        syscall(Syscall::Open {
            path_ptr: &(path.as_bytes()[0]) as *const u8 as usize,
            path_len: path.len(),
            mode: 0,
            ret_ptr: &mut ret_fd as *mut usize as usize
        });
        if ret_fd != 0 {
            return Some(ret_fd);
        }
    }
    
    None
}

pub fn fread(fd: usize, buffer: &mut [u8]) -> usize {
    let mut bytes_read: usize = 0;
    syscall(Syscall::Read {
            fd: fd,
            buf_ptr: &mut buffer[0] as *mut u8 as usize,
            buf_len: buffer.len(),
            ret_ptr: &mut bytes_read as *mut usize as usize
    });
    bytes_read
}

pub fn fwrite(fd: usize, buffer: &[u8]) -> usize {
    let mut bytes_written: usize = 0;
    syscall(Syscall::Write {
            fd: fd,
            buf_ptr: &buffer[0] as *const u8 as usize,
            buf_len: buffer.len(),
            ret_ptr: &mut bytes_written as *mut usize as usize
    });
    bytes_written
}

pub fn fseek(_fd: usize, _cursor: FileSeekCursor, _origin: FileSeekOrigin,
                _offset: isize ) -> usize {
    // let mut new_pos: usize = 0;
    // syscall(Syscall::Seek {
    //         fd: fd,
    //         offset: offset,
    //         cursor: cursor as usize,
    //         ret_ptr: &mut new_pos as *mut usize as usize
    // });
    // new_pos
    0
}

pub fn fenum(fd: usize, buffer: &mut [u8]) -> usize {
    let mut bytes_read: usize = 0;
    syscall(Syscall::Enum {
            fd: fd,
            buf_ptr: &mut buffer[0] as *mut u8 as usize,
            buf_len: buffer.len(),
            ret_ptr: &mut bytes_read as *mut usize as usize
    });
    bytes_read
}

pub fn fexec(fd: usize, func: usize, buffer: &mut [u8]) -> usize {
    let mut retval: usize = func;
    syscall(Syscall::Exec {
            fd: fd,
            cmd_buf_ptr: &mut buffer[0] as *mut u8 as usize,
            cmd_buf_len: buffer.len(),
            ret_ptr: &mut retval as *mut usize as usize
    });
    retval
}

pub fn fclose(fd: usize) {
    syscall(Syscall::Close { fd });
}

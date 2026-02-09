//
// Filesystem
//

use crate::syscall::*;

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

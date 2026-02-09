// 
// BlightOS System Call Inteface - Top-level structures
//
use core::arch::asm;

#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum SyscallOpCode {
    TaskCtl         = 0,
    Open            = 1,
    Enum            = 2,
    Read            = 3,
    Write           = 4,
    Exec            = 5,
    Close           = 6,
    Max             = 7
}

pub enum Syscall {
    // Task/Process control
    TaskControl{opcode: usize, args: usize, ret_code: usize},

    // Device/File control
    Open{path_ptr: usize, path_len: usize, mode: usize, ret_ptr: usize},
    // Enum returns file/directory/device information
    Enum{fd: usize, buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Read{fd: usize, buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Write{fd: usize, buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Exec{fd: usize, cmd_buf_ptr: usize, cmd_buf_len: usize, ret_ptr: usize},
    Close{fd: usize},
}

#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum SyscallRsvdFDs {
    StandardIO      = 0,
    StandardError   = 1,
    // Reading from this file returns string name prefixes that can be used to
    // access mount points and various devices, e.g., disk0.0, uart2, kbd0, etc.
    SystemResources = 2,
}

//
// Task Control structures
//
#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum TaskControlOpCode {
    Exit        = 0,
    Current     = 1,
    Spawn       = 2,
    Join        = 3,
}

#[repr(C, packed)]
pub struct TaskControlCurrentArguments{
    pub tid:        usize,
    pub pid:        usize,
    pub name:       [u8; 64]
}



//
// Low level system call interface
//

pub fn syscall(params: Syscall) {
    match params {
        Syscall::TaskControl {opcode, args, ret_code}                   => {
            syscall_trigger_int(SyscallOpCode::TaskCtl as usize,
                                opcode, args, ret_code, 0);
        },
        Syscall::Open { path_ptr, path_len, mode, ret_ptr }             => {
            syscall_trigger_int(SyscallOpCode::Open as usize,
                                path_ptr, path_len, mode, ret_ptr);
        },
        Syscall::Enum { fd, buf_ptr, buf_len, ret_ptr }                 => {
            syscall_trigger_int(SyscallOpCode::Enum as usize,
                                fd, buf_ptr, buf_len, ret_ptr);
        }
        Syscall::Read { fd, buf_ptr, buf_len, ret_ptr }                 => {
            syscall_trigger_int(SyscallOpCode::Read as usize,
                                fd, buf_ptr, buf_len, ret_ptr);
        },
        Syscall::Write { fd, buf_ptr, buf_len, ret_ptr }                => {
            syscall_trigger_int(SyscallOpCode::Write as usize,
                                fd, buf_ptr, buf_len, ret_ptr);
        },
        Syscall::Exec { fd, cmd_buf_ptr, cmd_buf_len, ret_ptr }         => {
            syscall_trigger_int(SyscallOpCode::Exec as usize,
                                fd, cmd_buf_ptr, cmd_buf_len, ret_ptr);
        },
        Syscall::Close { fd }                                           => {
            syscall_trigger_int(SyscallOpCode::Close as usize , fd, 0, 0, 0);
        }
    }
}

fn syscall_trigger_int(opcode: usize,
                        arg0: usize, arg1: usize, arg2: usize, arg3: usize) {
    unsafe{
        asm!(
            "int 0x20", // See boot.S
            in("rax") opcode,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            in("rcx") arg3,
        );
    };
}
// 
// BlightOS System Call Inteface - Top-level structures
//
use core::arch::asm;

#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum SyscallOpCode {
    TaskCtl         = 0,
    ProcCtl         = 1,
    Open            = 2,
    Enum            = 3,
    Read            = 4,
    Write           = 5,
    Exec            = 6,
    Close           = 7,
    Max             = 8
}

pub enum Syscall {
    // Task/Process control
    TaskControl{opcode: usize, args: usize, ret_code: usize},
    ProcControl{opcode: usize, args: usize, ret_code: usize},

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
    CurrentCpu  = 2,
    Spawn       = 3,
    Join        = 4,
    Yield       = 5,
    Sleep       = 6,
}

#[repr(C, packed)]
pub struct TaskControlCurrentArguments{
    pub tid:        usize,
    pub pid:        usize,
    pub name:       [u8; 64]
}

#[repr(C, packed)]
pub struct TaskControlSpawnArguments{
    pub func_ptr:   usize,          // Input
    pub func_arg:   usize,          // Input
    pub name:       [u8; 64],       // Input
    pub name_len:   usize,          // Derived
    pub tid:        usize,          // Output
    pub pid:        usize,          // Output
}

#[repr(C, packed)]
pub struct TaskControlJoinArguments{
    pub tid:        usize,          // Input
    pub joined:     bool,           // Output
}


//
// Process Control structures
//
#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum ProcCtlOpCode {
    Exit        = 0,
    Current     = 1, // Returns the PID and TID of the main task of the current process
    GetInfo     = 2, // Returns more detailed information about the current process
    ResizeHeap  = 3, // Expand/Shrink the heap of the current process
    Spawn       = 4, // Spawn a new process by executing a file
    Fork        = 5, // Clone the current process
    Exec        = 6, // Replace the current process with a new executable
}

pub struct ProcCtlCurrentArgs{
    pub pid:        usize,
    pub main_tid:   usize,
}

pub struct ProcCtlGetInfoArgs {
    pub pid:                usize,
    pub name:               [u8; 64],
    pub main_tid:           usize,
    pub task_count:         usize,
    pub fd_count:           usize,
    pub img_base:           usize,
    pub img_size:           usize,
    pub heap_base:          usize,
    pub heap_size:          usize,
    pub stack_top:          usize,
    pub total_mem_usage:    usize,
    pub meta_mem_usage:     usize,
}

pub struct ProcCtlSpawnArgs {
    pub path_ptr: usize,// Input: Pointer to the path string in user-space
    pub path_len: usize,// Input: Length of the path string
    // pub cmd_ptr: usize, // Input: Pointer to the command buffer in user-space (optional)
    // pub cmd_len: usize, // Input: Length of the command buffer (optional)
    // pub env_ptr: usize, // Input: Pointer to the environment variables buffer in user-space (optional)
    // pub env_len: usize, // Input: Length of the environment variables buffer (optional)
    pub pid:    usize,  // Output
    pub m_tid:  usize,  // Output TID of the main task
}

pub struct ProcCtlResizeHeapArgs {
    pub delta:      isize,  // Input: Positive to expand, Negative to shrink
    pub heap_base:  usize,  // Output: New heap base (No change after the initial expansion)
    pub heap_size:  usize   // Output: New heap size
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
        Syscall::ProcControl {opcode, args, ret_code}                   => {
            syscall_trigger_int(SyscallOpCode::ProcCtl as usize,
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

#[cfg(target_arch = "x86_64")]
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

#[cfg(target_arch = "aarch64")]
fn syscall_trigger_int(opcode: usize,
                       arg0: usize, arg1: usize, arg2: usize, arg3: usize) {
    unsafe{
        asm!(
            "svc #1000", // See boot.S
            in("x0") opcode,
            in("x1") arg0,
            in("x2") arg1,
            in("x3") arg2,
            in("x4") arg3,
        );
    };
}
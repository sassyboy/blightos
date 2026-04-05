// 
// BlightOS System Call Inteface - Top-level structures
//
use core::arch::asm;

#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum SyscallOpCode {
    TaskCtl         = 0,
    ProcCtl         = 1,
    TimeCtl         = 2,
    Open            = 3,
    Enum            = 4,
    Read            = 5,
    Write           = 6,
    Exec            = 7,
    Close           = 8,
    Max             = 9
}

pub enum Syscall {
    // Task/Process control
    TaskControl{opcode: usize, args: usize, ret_code: usize},
    ProcControl{opcode: usize, args: usize, ret_code: usize},
    TimeControl{opcode: usize, args_ptr: usize, args_len: usize, ret_ptr: usize},
    // Virtual File System - Device/File control
    // Open returns a File object to the user-space which includes a file
    // descriptor (FD) and some basic information about the directory entry of
    // the file such as attributes, size, etc. The calling process adress space
    // will own the original file object if successful.
    Open{args_ptr: usize, args_len: usize, arg3: usize, ret_ptr: usize},
    // Enum returns file/directory/device information
    Enum{args_ptr: usize, args_len: usize, arg3: usize, ret_ptr: usize},
    // Reads buf_len bytes of data from the file/device into the buffer pointed
    // by buf_ptr
    Read{args_ptr: usize, args_len: usize, arg3: usize, ret_ptr: usize},
    // Writes buf_len bytes of data from the buffer pointed by buf_ptr into the
    // file/device
    Write{args_ptr: usize, args_len: usize, arg3: usize, ret_ptr: usize},
    // Executes a command on the file/device represented by fd with the command
    Exec{args_ptr: usize, args_len: usize, arg3: usize, ret_ptr: usize},
    // Closes the file/device represented by fd
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
    pub cmd_line:           [u8; 1024],
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
    pub cmd_ptr: usize, // Input: Pointer to the cmd-line string in user-space
    pub cmd_len: usize, // Input: Length of the cmd-line string
    pub pid:    usize,  // Output
    pub m_tid:  usize,  // Output TID of the main task
}

pub struct ProcCtlResizeHeapArgs {
    pub delta:      isize,  // Input: Positive to expand, Negative to shrink
    pub heap_base:  usize,  // Output: New heap base (No change after the initial expansion)
    pub heap_size:  usize   // Output: New heap size
}

//
// Time Control structures
//
#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum TimeCtlOpCode {
    GetTscFreq     = 0, // Returns the current TSC frequency in Hz
    GetRealTime    = 1, // Returns the current real time in UNIX timestamp
    SetRealTime    = 2, // Sets the current real time with a UNIX timestamp
}
#[repr(C, packed)]
pub struct TimeCtlTscFreqArgs {
    pub tsc_freq_hz: u64, // Output: Current TSC frequency in Hz
}

//
// VFS System call structures
//
#[repr(C, packed)]
pub struct VfsOpenArgs {
    // Inputs from user-space
    pub path_ptr: usize,  // Pointer to the file path string in user-space
    pub path_len: usize,  // Length of the file path string
    pub mode:     usize,  // A combination of File::MODE_* flags
    // Output to user-space
    pub fd:       usize,  // File descriptor for the opened file (0 if failed)
    pub attr:     usize,  // A combination of DirectoryEntry::FLG_* flags
    pub size:     usize,  // Size of the file in bytes (0 if failed)
}

#[repr(C, packed)]
pub struct VfsEnumArgs {
    // Inputs from user-space
    pub fd:       usize,  // The target file/directory/device to enumerate.
    pub buf_ptr:  usize,  // Pointer to the output buffer in user-space
    pub buf_len:  usize,  // Length of the output buffer
    pub skip:     usize,  // Number of entries to skip for pagination 
                          // (0 for the first call)
    // Output to user-space:
    pub count:    usize,  // Number of entries enumerated (0 if failed)
}

#[repr(C, packed)]
pub struct VfsReadWriteArgs {
    pub fd:       usize,  // The target file/device to read/write
    pub offset:   usize,  // Offset in the file/device to read/write
    pub buf_ptr:  usize,  // Pointer to the buffer in user-space
    pub buf_len:  usize,  // Length of the buffer
    // Output to user-space:
    pub bytes:    usize,  // Number of bytes actually read/written
}

#[repr(C, packed)]
pub struct VfsExecArgs {
    pub fd:         usize,  // The target file/device to execute
    pub func_code:  usize,  // The command/function code to execute
    pub args_ptr:   usize,  // Pointer to the command buffer in user-space
    pub args_len:   usize,  // Length of the command buffer
    // Output to user-space
    pub ret_val:    usize,  // Return value from the executed command
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
        Syscall::TimeControl {opcode, args_ptr, args_len, ret_ptr}     => {
            syscall_trigger_int(SyscallOpCode::TimeCtl as usize,
                                opcode, args_ptr, args_len, ret_ptr);
        },
        Syscall::Open { args_ptr, args_len, arg3, ret_ptr }             => {
            syscall_trigger_int(SyscallOpCode::Open as usize,
                                args_ptr, args_len, arg3, ret_ptr);
        },
        Syscall::Enum { args_ptr, args_len, arg3, ret_ptr }             => {
            syscall_trigger_int(SyscallOpCode::Enum as usize,
                                args_ptr, args_len, arg3, ret_ptr);
        },
        Syscall::Read { args_ptr, args_len, arg3, ret_ptr }             => {
            syscall_trigger_int(SyscallOpCode::Read as usize,
                                args_ptr, args_len, arg3, ret_ptr);
        },
        Syscall::Write { args_ptr, args_len, arg3, ret_ptr }            => {
            syscall_trigger_int(SyscallOpCode::Write as usize,
                                args_ptr, args_len, arg3, ret_ptr);
        },
        Syscall::Exec { args_ptr, args_len, arg3, ret_ptr }             => {
            syscall_trigger_int(SyscallOpCode::Exec as usize,
                                args_ptr, args_len, arg3, ret_ptr);
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
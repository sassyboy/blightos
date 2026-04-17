//
// BlightOS Kernel
//
// Root Module
//
// 
#![no_std]
#![no_main]
#![feature(linked_list_retain)]

// Imports from the toolchain this is part of the toolchain
extern crate alloc; 

// Include all relevant kernel code here for everybody else to use
//// Standard utilities ////
#[macro_use]
pub mod util;
//// Architecture-dependent code ////
pub mod arch;
//// Various Memory Managers ////
pub mod mem;
//// Task Scheduler ////
pub mod sched;
//// File System /////
pub mod fs;
//// Device Drivers ////
pub mod drivers;

pub mod test;

use core::cmp::min;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use alloc::boxed::Box;
use alloc::{format, str};
use alloc::string::{String, ToString};
use util::*;
use crate::arch::*;
use crate::drivers::storage::num_disks;
use crate::fs::*;
use crate::mem::phys::*;
use crate::mem::virt::AddressSpace;
use crate::sched::{SCHEDULER, Scheduler, Task};
use crate::drivers::input::Keyboard;

#[cfg(feature="debug_kern")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[KERN] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_kern"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}

unsafe extern "C" {
    unsafe static _KERNEL_START: usize;
    unsafe static _KERNEL_END: usize;
}

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum ErrorCode {
    NoError             = 0,    // No error, the operation was successful
    OutOfMemory         = 1,    // Ran out of memory
    NotSupported        = 2,   // Feature/Operation not supported/implemented
    NotFound            = 3,    // The requested resource was not found
    NotIssued           = 4,    // The request couldn't be issued
    UnexpectedEoF       = 5,    // An unexpected end of file was reached
    InvalidPID          = 6,    // Invalid Process ID
    InvalidTID          = 7,    // Invalid Task ID
    InvalidFD           = 8,    // Invalid File Descriptor
    InvalidOp           = 9,    // Invalid operation for the target resource
    InvalidBus          = 10,    // Invalid Bus (e.g., AHCI[1])
    InvalidDrive        = 11,   // Invalid drive/device
    InvalidMountPoint   = 12,   // Invalid mount point
    InvalidPath         = 13,   // Invalid path
    InvalidHandle       = 14,   // Invalid device handle
    InvalidBuffer       = 15,   // Invalid buffer pointer/length
    InvalidArgument     = 16,   // Invalid argument
    InvalidFormat       = 17,   // Invalid file/data/buffer/etc. format
    NotAllowed          = 18,   // The operation is not permitted on the resource
    OutOfBoundIO        = 19,   // The IO is out-of-bound for the resource
    IOError             = 20,   // An error occurred during IO on the resource
    Other               = 255,  // Other errors
}
#[derive(Clone, Copy, Debug)]
pub struct Error {
    pub code: ErrorCode,
    pub file: &'static str,
    pub line: u32,
    pub message: &'static str,
}
impl Error {
    pub const fn new(code: ErrorCode) -> Self {
        Self { code, file: "", line: 0, message: "" }
    }
}


static BSP_INITIALIZED : AtomicBool = AtomicBool::new(false);

#[global_allocator]
static ALLOCATOR: mem::heap::Kalloc = mem::heap::Kalloc::new();

pub struct RamdiskInfo {
    // Start/End of the physical address where the image is copied
    pub start_phy_addr: usize,
    pub end_phy_addr:   usize
}

// kstart : Kernel's Generic Entry Point
// This function will be called by all onlined CPUs (BSP with cpuid=0) in any
// order, albeit only after all onlined CPUs have reported to the arch-specific
// stub code so that the generic code has the correct CPU count (e.g., for 
// resource allocation purposes).
pub fn kstart(cpuid: usize, mmap_opt: Option<&[PMMapElement]>)
{
    if cpuid == 0 {
        // BSP-only initialization
        klog!("BlightOS - Number of CPUs online: {}\n", cpu_count());

        // Initialize the physical memory manage
        match mmap_opt {
            Some(mmap) => {
                let kernel_start: usize;
                let kernel_end: usize;
                unsafe{
                    kernel_start = &_KERNEL_START as *const usize as usize;
                    kernel_end = &_KERNEL_END as *const usize as usize;
                }
                // Initialize the physical memory manager. Also marks the kernel
                // and the initramdisk (if any) as used - No ramdisk anymore
                PhysMem::init(mmap, kernel_start, kernel_end, None);
            },
            _ => {panic!("No memory map was sent to the BSP!")}
        }

        // Load the drivers
        let drvs = drivers::get_builtin_drivers();
        for d in drvs.iter() {
            let ndevs = (d.enumerate)();
            klog!("Built-in Driver: {} - Enumerated {} device(s)\n",
                d.name, ndevs);
        }

        // Time keeping...
        SystemTimer::global_init(systimer_irq_handler);
        SystemTimer::per_cpu_init();
        // Todo: Need an event timer to implement sleep, etc.

        // Install the system call handlers
        arch::syscall_register(SyscallOpCode::TaskCtl,  syscall_task_control);
        arch::syscall_register(SyscallOpCode::ProcCtl,  syscall_proc_control);
        arch::syscall_register(SyscallOpCode::TimeCtl,  syscall_time_control);
        arch::syscall_register(SyscallOpCode::Open,     syscall_open);
        arch::syscall_register(SyscallOpCode::Enum,     syscall_enum);
        arch::syscall_register(SyscallOpCode::Read,     syscall_read);
        arch::syscall_register(SyscallOpCode::Write,    syscall_write);
        arch::syscall_register(SyscallOpCode::Exec,     syscall_exec);
        arch::syscall_register(SyscallOpCode::Close,    syscall_close);
        
        // BSP initialization finished. Unblock APs, start the scheduler,
        // and jump to the first task
        Scheduler::config_round_robin(Duration::from_micros(200));
        // Scheduler::config_round_robin(Duration::from_millis(1000));

        // Spawn the first process address space from shell.elf.
        // Scans the enumerated partitions for the following path:
        // /blightos/shell.elf
        Task::spawn_on_cpu(kinit_task, 0, cpuid, String::from("kInit"));
        BSP_INITIALIZED.store(true, Ordering::Relaxed);
        SCHEDULER.borrow_mut().start_scheduling();
    } else {
        // AP initialization
        // Wait for BSP to perform the serialized portion of
        // kernel's initializaton
        while BSP_INITIALIZED.load(Ordering::Relaxed) == false {
            core::hint::spin_loop();
        }
        SystemTimer::per_cpu_init();
        // Jump to the first task!
        Scheduler::config_round_robin(Duration::from_micros(200));
        SCHEDULER.borrow_mut().start_scheduling();
    }
    panic!("Reached the end of kstart!");
}

fn systimer_irq_handler(_cpuid: u16){
    SCHEDULER.borrow_mut().preempt_irq();
}

#[panic_handler]
pub fn panic(_info: &core::panic::PanicInfo) -> ! {
    arch::cpu_disable_ints();

    klog!("KERNEL BUG: {}\n", _info.message());

    let (fname, ln) = match  _info.location() {
        Some(loc) => (loc.file(), loc.line()),
        None => ("Unknown", 0)
    };
    // All paths start with "kernel/src/", so skip the first 11 chars
    if fname.len() > 11 {
        klog!("  LOCATION: {} @ ln {}\n", &fname[11..], ln);
    } else {
        klog!("  LOCATION: {} @ ln {}\n", fname, ln);
    }
    klog!("  CPU: {}\n", arch::cpu_id());
    arch::cpu_halt();
    loop {}
}


//////
/// The INIT task
/// 1) Initializes the drivers' post-enum stage
/// 2) (Optional) Performs kernel's self-test
/// 3) Sets up the first address-space and lunches the first user program
///
fn kinit_task(_arg: usize) {
    // Perform the post-enumeration phase of the drivers
    dbg!("[{}({})] started on CPU {} (Free frames: {})\n", 
            Task::name(), Task::current_tid(),Task::current_cpu(),
            PhysMem::free_frame_count());
    let drvs = drivers::get_builtin_drivers();
    for d in drvs.iter() {
        (d.post_enum)();
    }

    let ndisks = num_disks();
    for d in 0..ndisks {
        dbg!("Preparing disk {} out of {}\n", d + 1, ndisks);
        enumerate_filesystems(d);
    }
    
    // Kernel Self-Test - Moved to an fexec call on the machine: file
    // klog!("calling test::kself_test\n");
    // test::kself_test();

    // Find the initial binary (disk%d.0:/blightos/shell.box) to load
    for d in 0..ndisks {
        let path = format!("disk{}.0:/blightos/shell.box", d);
        // Open & parse the ELF file if exists
        let Ok(elf) = ELFBinary::from_path(path.as_str()) else {
            continue;
        };
        dbg!("  Found INIT program at {} on disk {} - Free frames: {}\n", 
                path.as_str(), d, PhysMem::free_frame_count());
        // elf.log_header();
        // klog!("Segments:\n");
        // for i in 0..elf.segments.len() {
        //     klog!("  {:X?}\n", elf.segments[i]);
        // }

        // Spawn a new process address space
        let pname = "shell.box".to_string();
        let pspwn = AddressSpace::spawn(pname, path.clone());
        let Ok(pid) = pspwn else {
            let e = pspwn.err().unwrap();
            klog!("  Failed to launch the INIT process from {} due to {:?}\n",
                    path.as_str(), e);
            continue;
        };
        dbg!("  {}({}) is launching the INIT process ({}) - Free frames: {}\n",
                        Task::name(), Task::current_tid(), pid,
                        PhysMem::free_frame_count());
        Keyboard::clear_buffer();
        let _ = AddressSpace::launch_elf(pid, elf).unwrap_or_else(|e| {
            klog!("  Failed to launch the INIT process due to {:?}\n", e);
        });
        panic!("Unreachable end of kinit_task!");
    }
    panic!("/blightos/shell.box not found on any supported disk partition!");
    
}


///////
/// User-space interface
/// 

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

#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum SyscallRsvdFDs {
    StandardIO      = 0,
    StandardError   = 1,
    // Reading from this file returns string name prefixes that can be used to
    // access mount points and various devices, e.g., disk0.0, uart2, kbd0, etc.
    SystemResources = 2,
    Max             = 3,
}

pub type SyscallHandlerFn = fn(usize, usize, usize, usize);

fn copy_to_user<T>(dst_ptr: usize, ret_val: T) {
    if dst_ptr != 0 {
        unsafe {(dst_ptr as *mut T).write(ret_val);}
    }
}

fn copy_from_user<T>(src_ptr: usize) -> Option<T> {
    if src_ptr == 0 {
        None
    } else {
        Some(unsafe { (src_ptr as *const T).read() })
    }
}

//
// Task Control System Call
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

struct UserTaskLaunchInfo {
    func_ptr: fn(usize),
    func_arg: usize,
    target_pid: usize,
}

fn kuser_task_launcher(args: usize) {
    let info = unsafe { &*(args as *const Box<UserTaskLaunchInfo>) };
    let pid = info.target_pid;
    let func = info.func_ptr;
    let farg = info.func_arg;
    dbg!("kuser_task_launcher: PID={}, func={:p}, arg={}\n",
            info.target_pid, info.func_ptr, info.func_arg);
    // Box will be dropped here
    if let Err(e) = AddressSpace::move_to_process(pid, func, farg) {
        klog!("move_to_process(PID={}) failed  due to {:?}\n", pid, e);
        return;
    }
    klog!("BUG: TID{} returned to kuser_task_launcher!\n", Task::current_tid());
    Task::exit();
}

fn syscall_task_control(opcode: usize, args: usize, ret_ptr: usize, _: usize) {
    if opcode == TaskControlOpCode::Exit as usize {
        //
        // EXIT
        //
        klog!("\nProgram ({} - MainTID:{}) Exited with status {}\n",
                                    Task::name(), Task::current_tid(), args);
        Task::exit();
    } else if opcode == TaskControlOpCode::CurrentCpu as usize {
        // CURRENT CPU
        copy_to_user(ret_ptr, Task::current_cpu());
    }else if opcode == TaskControlOpCode::Current as usize {
        //
        // CURRENT
        //
        let ret_val: usize;
        if args == 0 || ret_ptr == 0 {
            // Invalid arguments
            ret_val = 0;
        } else {
            let mut tname_out = [0 as u8; 64];
            let tname = Task::name();
            let tname_len = min(tname.len(), 63);
            tname_out[0..tname_len].copy_from_slice(
                                        &tname.as_bytes()[0..tname_len]);
            copy_to_user(args, TaskControlCurrentArguments {
                                    tid: Task::current_tid(),
                                    pid: Task::current_pid(),
                                    name: tname_out});
            ret_val = size_of::<TaskControlCurrentArguments>();
        }
        // Return to user-space
        copy_to_user(ret_ptr, ret_val);
    } else if opcode == TaskControlOpCode::Spawn as usize {
        //
        // SPAWN (Creates a task in the current process address space)
        //
        let mut ret_val: usize = 0;
        if args == 0 || ret_ptr == 0 {
            // Invalid arguments
        } else if let Some(mut info) = copy_from_user::<TaskControlSpawnArguments>(args) {
            if info.name_len > 64 || info.func_ptr == 0 {
                // Invalid name length
                ret_val = 0;
            } else {
                // Valid arguments - Perform the spawn and return the TID/PID back
                let name_bytes = &info.name[0..info.name_len];
                let name_str;
                if info.name_len > 0 {
                    name_str = str::from_utf8(name_bytes).unwrap_or("UnnamedTask");
                } else {
                    name_str = "UnnamedTask";
                }
                let fn_ptr : fn(usize) = unsafe {
                    core::mem::transmute(info.func_ptr)
                };
                let launch_args = Box::new(UserTaskLaunchInfo {
                    func_ptr: fn_ptr,
                    func_arg: info.func_arg,
                    target_pid: Task::current_pid()
                });
                let new_tid = Task::spawn_named(kuser_task_launcher, 
                        &launch_args as *const Box<UserTaskLaunchInfo> as usize,
                        name_str.to_string());
                if new_tid != 0{
                    info.tid = new_tid;
                    info.pid = Task::current_pid();
                    copy_to_user(args, info);
                    ret_val = size_of::<TaskControlSpawnArguments>();
                }
            }
        }
        copy_to_user(ret_ptr, ret_val);
    } else if opcode == TaskControlOpCode::Join as usize {
        //
        // JOIN (Joins a task in the current process address space)
        //
        let mut ret_val: usize = 0;
        if args == 0 || ret_ptr == 0 {
            // Invalid arguments
        } else if let Some(mut info) = copy_from_user::<TaskControlJoinArguments>(args) {
            if Task::exists(info.tid) {
                // Valid TID - Perform the join and return success
                Task::join(info.tid);
                info.joined = true;
            } else {
                // Invalid TID
                info.joined = false;
            }
            copy_to_user(args, info);
            ret_val = size_of::<TaskControlJoinArguments>();
        }
        copy_to_user(ret_ptr, ret_val);
    } else if opcode == TaskControlOpCode::Yield as usize {
        //
        // YIELD (Yields the current task's execution voluntarily)
        //
        Task::preempt();
        // No return value for yield
    } else if opcode == TaskControlOpCode::Sleep as usize {
        //
        // SLEEP (Puts the current task to sleep for a specified duration)
        //
        if args == 0 {
            // Invalid arguments
        } else if let Some(info) = copy_from_user::<Duration>(args) {
            Task::sleep(info);
        }
        // No return value for sleep
    }else {
        klog!("\nInvalid TaskControlOpcode\n");
    }
    
}

//
// Process Control System Call
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
    pub pid:     usize, // Output: PID of the spawned process
    pub m_tid:   usize, // Output: TID of the main task
}

pub struct ProcCtlResizeHeapArgs {
    pub delta:      isize,  // Input: Positive to expand, Negative to shrink
    pub heap_base:  usize,  // Output: New heap base (No change after the initial expansion)
    pub heap_size:  usize   // Output: New heap size
}

struct KUserProcLaunchArgs {
    pid: usize,
    elf: ELFBinary,
}

fn kuser_proc_launcher(args: usize) {
    let info = unsafe { Box::from_raw(args as *mut KUserProcLaunchArgs) };
    dbg!("kuser_proc_launcher: PID={} by {}(TID={})\n", info.pid,
            Task::name(), Task::current_tid());
    // Spawn a new process address space
    let pid = info.pid;
    let elf = info.elf.clone();
    drop(info);
    if let Err(e) = AddressSpace::launch_elf(pid, elf) {
        klog!("Failed to launch the user-space process for PID {}: {:?}\n",
                pid, e);
        return;
    }
    klog!("BUG: TID{} returned to kuser_proc_launcher!\n", Task::current_tid());
    Task::exit();
}

fn syscall_proc_control(opcode: usize, args: usize, ret_ptr: usize, _: usize) {
    if opcode == ProcCtlOpCode::Exit as usize {
        //
        // EXIT
        //
        let exit_code = args;
        klog!("\nProcess {} Exited with status {}\n", Task::current_pid(),
                                                                    exit_code);
        // TODO: Implement - Kill the main task
    } else if opcode == ProcCtlOpCode::Current as usize {
        //
        // CURRENT
        //
        if args != 0 && ret_ptr != 0 {
            let pid = Task::current_pid();
            if let Some(mtid) = AddressSpace::get_main_tid(pid) {
                copy_to_user(args, ProcCtlCurrentArgs {
                                        pid: pid,
                                        main_tid: mtid
                });
                copy_to_user(ret_ptr, size_of::<ProcCtlCurrentArgs>());
                return;
            }
            copy_to_user(ret_ptr, 0);
        }
    } else if opcode == ProcCtlOpCode::GetInfo as usize {
        //
        // GET INFO
        //
        let mut ret_val: usize = 0;
        if args != 0 && ret_ptr != 0 {
            let pid = Task::current_pid();
            if let Some(info) = AddressSpace::get_process_info(pid) {
                copy_to_user(args, info);
                ret_val = size_of::<ProcCtlGetInfoArgs>();
            }
        }
        copy_to_user(ret_ptr, ret_val);
    } else if opcode == ProcCtlOpCode::Spawn as usize {
        //
        // SPAWN
        // Creates a new process and a new task (main) for the process, and
        // offloads the actual load & launch to the task to avoiding spending
        // too long in the syscall handler.
        //
        let Some(mut info) = copy_from_user::<ProcCtlSpawnArgs>(args) else {
            // Invalid arguments pointer
            return;
        };
        // Extract the ELF path and executable name from the command line string
        let cmd_line;
        let path;
        let pname;
        if info.cmd_ptr == 0 || info.cmd_len == 0 {
            // Null/Empty command line
            copy_to_user(ret_ptr, 0);
            return;
        }
        let cmd_bytes = unsafe {
            core::slice::from_raw_parts(info.cmd_ptr as *const u8, info.cmd_len)
        };
        cmd_line = str::from_utf8(cmd_bytes).unwrap_or("");
        if cmd_line.len() == 0 {
            // Invalid command line
            copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
            return;
        }
        path = match cmd_line.find(' ') {
            Some(idx) => &cmd_line[0..idx],
            None => cmd_line
        };
        pname = match path.rfind('/') {
            Some(idx) => &path[idx + 1..],
            None => path
        };
        // Open/parse the ELF file
        let elf_result = ELFBinary::from_path(path);
        let Ok(elf) = elf_result else {
            let e = elf_result.err().unwrap();
            // Invalid ELF file
            klog!("Failed to load the ELF file to spawn a process: {:?}\n", e);
            copy_to_user(ret_ptr, e.code);
            return;
        };

        // Create a new process
        let pname_str = pname.to_string();
        let pcmd_str = cmd_line.to_string();
        let pspwn_ret = AddressSpace::spawn(pname_str, pcmd_str);
        let Ok(pid) = pspwn_ret else {
            // Failed to create a new process
            let e = pspwn_ret.err().unwrap();
            klog!("Failed to spawn a new process for {}: {:?}\n", path, e);
            copy_to_user(ret_ptr, e.code);
            return;
        };
        
        // Create and launch the main task
        let launch_args = Box::into_raw(
            Box::new(KUserProcLaunchArgs {
                pid: pid,
                elf: elf
            })
        ) as usize;
        let tname = format!("P[{}].main", pid).to_string();
        info.m_tid = Task::spawn_named(kuser_proc_launcher, launch_args, tname);
        info.pid = pid;
        // Return the PID back to the caller
        copy_to_user(args, info);
        copy_to_user(ret_ptr, ErrorCode::NoError);
        return; // Success
    } else if opcode == ProcCtlOpCode::ResizeHeap as usize {
        //
        // RESIZE HEAP
        //
        let mut ret_val: usize = 0;
        if let Some(mut info) = copy_from_user::<ProcCtlResizeHeapArgs>(args) {
            dbg!("\nProcess {} requested heap resize with delta {}\n", 
                Task::current_pid(), info.delta);
            let delta = info.delta;
            if let Ok((new_base, new_size)) = AddressSpace::resize_heap(delta) {
                info.heap_base = new_base;
                info.heap_size = new_size;
                copy_to_user(args, info);
                ret_val = size_of::<ProcCtlResizeHeapArgs>();
            }
        }
        copy_to_user(ret_ptr, ret_val);
    } else {
        klog!("\nInvalid ProcessControlOpcode\n");
    }
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

fn syscall_time_control(opcode: usize, args_ptr: usize, args_len: usize,
                                                            ret_ptr: usize) {
    if ret_ptr == 0 {
        // Invalid return pointer
        return;
    } 
    if opcode == TimeCtlOpCode::GetTscFreq as usize {
        //
        // GET TSC FREQUENCY
        //
        if args_ptr == 0 || args_len != size_of::<TimeCtlTscFreqArgs>() {
            // Invalid arguments pointer/length
            copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
            return;
        }
        let args = TimeCtlTscFreqArgs {
            tsc_freq_hz: SystemTimer::frequency_hz(),
        };
        copy_to_user(args_ptr, args);
        copy_to_user(ret_ptr, ErrorCode::NoError);
    } else if opcode == TimeCtlOpCode::GetRealTime as usize {
        //
        // GET REAL TIME
        //
        copy_to_user(ret_ptr, ErrorCode::NotSupported); // TODO
    } else if opcode == TimeCtlOpCode::SetRealTime as usize {
        //
        // SET REAL TIME
        //
        copy_to_user(ret_ptr, ErrorCode::NotSupported); // TODO
    } else {
        copy_to_user(ret_ptr, ErrorCode::InvalidOp);
    }
}


//
// Virtual File System Control System Calls
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
    pub fd:       usize,  // The target file/directory/device to enumerate
    pub buf_ptr:  usize,  // Pointer to the output buffer in user-space
    pub buf_len:  usize,  // Length of the output buffer
    pub skip:     usize,  // Number of entries to skip for pagination 
                          // (0 for the first call)
    // Output to user-space:
    pub count:    usize,  // Number of entries enumerated
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

// ret_code = 0 no error
fn syscall_read_resources(out_buffer: &mut [u8], _offset: usize) -> usize {
    let mut out_index = 0;
    // Always return the disk%d.%d resources back
    let lst_parts = MountPoint::list_names();
    for mnt_name in lst_parts {
        if mnt_name.len() + 1 < out_buffer.len() - out_index {
            let name_bytes = mnt_name.as_bytes();
            out_buffer[out_index..(out_index+name_bytes.len())]
                                                .copy_from_slice(name_bytes);
            out_index += name_bytes.len();
            out_buffer[out_index] = b'\n';
            out_index += 1;
        }
    }
    // Remove the tailing \n if any
    if out_index > 0 && out_buffer[out_index - 1] == b'\n' {
        out_buffer[out_index - 1] = 0;
        out_index -= 1;
    }
    out_index
}

fn syscall_open(args_ptr: usize, args_len: usize, _: usize, ret_ptr: usize) {
    if ret_ptr == 0 {
        // Invalid return pointer
        return;
    } 
    if args_ptr == 0 || args_len != size_of::<VfsOpenArgs>() {
        // Invalid arguments pointer/length
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    }
    let Some(mut args) = copy_from_user::<VfsOpenArgs>(args_ptr) else {
        // Invalid arguments pointer
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    };
    let pathb;
    unsafe {
        pathb = core::slice::from_raw_parts(args.path_ptr as *const u8,
                                            args.path_len);
    }
    let Ok(path) = str::from_utf8(pathb) else {
        // Invalid path string
        copy_to_user(ret_ptr, ErrorCode::InvalidPath);
        return;
    };
    
    match File::open(path, args.mode) {
        Ok(file) => {
            let pid = Task::current_pid();
            args.attr = file.dir_entry.flags;
            args.size = file.dir_entry.size;
            let fd = AddressSpace::add_file(pid, file);
            args.fd = fd;
            copy_to_user(args_ptr, args);
            copy_to_user(ret_ptr, ErrorCode::NoError);
        },
        Err(e) => {
            // Failed to open the file
            dbg!("OPEN syscall failed for {} due to {:?}\n", path, e);
            copy_to_user(ret_ptr, e);
        }
    }
}

fn syscall_enum(args_ptr: usize, args_len: usize, _: usize, ret_ptr: usize) {
    if ret_ptr == 0 {
        // Invalid return pointer
        return;
    }
    if args_ptr == 0 || args_len != size_of::<VfsEnumArgs>() {
        // Invalid arguments pointer/length
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    }
    let Some(mut args) = copy_from_user::<VfsEnumArgs>(args_ptr) else {
        // Invalid arguments pointer
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    };
    if args.fd <= SyscallRsvdFDs::SystemResources as usize {
        //Enum not supported on special FDs!
        copy_to_user(ret_ptr, ErrorCode::NotAllowed);
        return;
    }

    let pid = Task::current_pid();
    match AddressSpace::get_file(pid, args.fd) {
        Ok(file) => {
            // Enum the file if it's a directory
            match file.enumerate() {
                Ok(entries) => {
                    let mut serialized = String::new();
                    // Todo - implement the skip mode
                    // Todo - return a well-structured data instead of a string
                    //args.count = entries.len();
                    for item in entries {
                        serialized += format!("{},{:X},{:X}\n",
                                    item.name, item.size, item.flags).as_str();
                    }
                    let len = min(args.buf_len, serialized.len());
                    unsafe {
                        let out = core::slice::from_raw_parts_mut(
                                                args.buf_ptr as *mut u8, len);
                        out[0..len].copy_from_slice(serialized.as_bytes());
                    }
                    args.count = len;
                    copy_to_user(args_ptr, args);
                    copy_to_user(ret_ptr, ErrorCode::NoError);
                },
                Err(e) => {
                    // Enum failed/not-supported
                    dbg!("ENUM syscall failed (PID: {}, FD: {}) due to {:?}\n",
                            e, pid, args.fd);
                    copy_to_user(ret_ptr, e.code);
                }
            }
        },
        Err(e) => {
            // Invalid PID/FD
            dbg!("ENUM syscall failed (PID: {}, FD: {}) due to {:?}\n",
                    e, pid, args.fd);
            copy_to_user(ret_ptr, e.code);
        }
    } 
}

fn syscall_read(args_ptr: usize, args_len: usize, _: usize, ret_ptr: usize) {
    if ret_ptr == 0 {
        // Invalid return pointer
        return;
    }
    if args_ptr == 0 || args_len != size_of::<VfsReadWriteArgs>() {
        // Invalid arguments pointer/length
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    }
    let Some(mut args) = copy_from_user::<VfsReadWriteArgs>(args_ptr) else {
        // Invalid arguments pointer
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    };
    if args.fd == SyscallRsvdFDs::StandardIO as usize && args.buf_len >= 1 {
        // Standard input - keyboard
        unsafe {
            // return the last keychar to the user-space
            let out = core::slice::from_raw_parts_mut(args.buf_ptr as *mut u8,
                                                                args.buf_len);
            out[0] = Keyboard::pop_ascii();
        }
        args.bytes = 1;
        copy_to_user(args_ptr, args);
        copy_to_user(ret_ptr, ErrorCode::NoError);
        return;
    }
    if args.fd == SyscallRsvdFDs::SystemResources as usize {
        // Resource Enumeration
        unsafe {
            let out = core::slice::from_raw_parts_mut(args.buf_ptr as *mut u8,
                                                                args.buf_len);
            let len = syscall_read_resources(out, 0);
            args.bytes = len;
        }
        copy_to_user(args_ptr, args);
        copy_to_user(ret_ptr, ErrorCode::NoError);
        return;
    }
    // Normal File/Device Read
    let pid = Task::current_pid();
    let fd = args.fd;
    match AddressSpace::get_file(pid, fd) {
        Ok(mut file) => {
            let seekr = file.seek(args.offset as isize, 
                            FileSeekOrigin::Start, FileSeekCursor::Read);
            if let Err(e) = seekr {
                // Invalid offset
                dbg!("READ syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, fd, e);
                copy_to_user(ret_ptr, e.code);
                return;
            };
            let rr;
            unsafe {
                let out = core::slice::from_raw_parts_mut(
                                        args.buf_ptr as *mut u8, args.buf_len);
                rr = file.read(out);
            };
            match rr {
                Ok(bytes_read) => {
                    args.bytes = bytes_read;
                    copy_to_user(args_ptr, args);
                    copy_to_user(ret_ptr, ErrorCode::NoError);
                },
                Err(e) => {
                    // Read failed
                    dbg!("READ syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, fd, e);
                    copy_to_user(ret_ptr, e.code);
                }
            }
        },
        Err(e) => {
            // Invalid PID/FD
            dbg!("READ syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, args.fd, e);
            copy_to_user(ret_ptr, e.code);
            return;
        }
    }
}


fn syscall_write(args_ptr: usize, args_len: usize, _: usize, ret_ptr: usize) {
    if ret_ptr == 0 {
        // Invalid return pointer
        return;
    }
    if args_ptr == 0 || args_len != size_of::<VfsReadWriteArgs>() {
        // Invalid arguments pointer/length
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    }
    let Some(mut args) = copy_from_user::<VfsReadWriteArgs>(args_ptr) else {
        // Invalid arguments pointer
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    };
    // Special FDs
    if args.fd == SyscallRsvdFDs::StandardIO as usize {
        // Standard output - VGA console
        unsafe {
            let buf = core::slice::from_raw_parts(args.buf_ptr as *const u8,
                                                    args.buf_len);
            // Todo - support cursor movement
            kearly_console::print_str(buf);
            args.bytes = args.buf_len;
        }
        copy_to_user(args_ptr, args);
        copy_to_user(ret_ptr, ErrorCode::NoError);
        return;
    }
    // Normal File Write
    let pid = Task::current_pid();
    let fd = args.fd;
    let offset = args.offset;
    let buf_len = args.buf_len;
    match AddressSpace::get_file(pid, fd) {
        Ok(mut file) => {
            let seekr = file.seek(offset as isize, 
                            FileSeekOrigin::Start, FileSeekCursor::Write);
            if let Err(e) = seekr {
                // Invalid offset
                dbg!("WRITE syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, fd, e);
                copy_to_user(ret_ptr, e.code);
                return;
            };
            let wr;
            unsafe {
                let buf = core::slice::from_raw_parts(args.buf_ptr as *const u8,
                                                                    buf_len);
                wr = file.write(buf);
            };
            match wr {
                Ok(bytes_written) => {
                    args.bytes = bytes_written;
                    copy_to_user(args_ptr, args);
                    copy_to_user(ret_ptr, ErrorCode::NoError);
                },
                Err(e) => {
                    // Write failed
                    dbg!("WRITE syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, fd, e);
                    copy_to_user(ret_ptr, e.code);
                }
            }
        },
        Err(e) => {
            // Invalid PID/FD
            dbg!("WRITE syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, fd, e);
            copy_to_user(ret_ptr, e.code);
            return;
        }
    }    
}

fn syscall_exec(args_ptr: usize, args_len: usize, _: usize, ret_ptr: usize) {
    if ret_ptr == 0 {
        // Invalid return pointer
        return;
    }
    if args_ptr == 0 || args_len != size_of::<VfsExecArgs>() {
        // Invalid arguments pointer/length
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    }
    let Some(mut args) = copy_from_user::<VfsExecArgs>(args_ptr) else {
        // Invalid arguments pointer
        copy_to_user(ret_ptr, ErrorCode::InvalidArgument);
        return;
    };
    let pid = Task::current_pid();
    // Special FDs
    if args.fd == SyscallRsvdFDs::StandardIO as usize {
        // Exec the command on the shell
        if args.func_code == 1 {
            // Clear screen
            kearly_console::init();
            copy_to_user(ret_ptr, ErrorCode::NoError);
        } else {
            copy_to_user(ret_ptr, ErrorCode::InvalidOp);
        }
        return;
    }
    // Normal Device File Command Execution
    match AddressSpace::get_file(pid, args.fd) {
        Ok(mut file) => {
            let cmd_buf;
            unsafe {
                cmd_buf = core::slice::from_raw_parts_mut(
                                    args.args_ptr as *mut u8, args.args_len);
            }
            let exr = file.execute(args.func_code, cmd_buf);
            match exr {
                Ok(func_ret) => {
                    args.ret_val = func_ret;
                    copy_to_user(args_ptr, args);
                    copy_to_user(ret_ptr, ErrorCode::NoError);
                },
                Err(e) => {
                    // Exec failed/not-supported
                    dbg!("EXEC syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, args.fd, e);
                    copy_to_user(ret_ptr, e.code);
                }
            }
        },
        Err(e) => {
            // Invalid PID/FD
            dbg!("EXEC syscall failed (PID: {}, FD: {}) due to {:?}\n",
                                                            pid, args.fd, e);
            copy_to_user(ret_ptr, e.code);
        }
    }
}

fn syscall_close(fd: usize, _: usize, _: usize, _: usize) {
    let pid = Task::current_pid();
    AddressSpace::remove_file(pid, fd);
}

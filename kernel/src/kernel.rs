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
use alloc::vec::Vec;
use util::*;
use crate::arch::*;
use crate::drivers::storage::{IOCompletion, num_disks};
use crate::fs::{DirectoryEntry, MountPoint, enumerate_filesystems};
use crate::mem::virt::{AddressSpace, FileDescriptor};
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
    static _KERNEL_START: usize;
    static _KERNEL_END: usize;
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
pub fn kstart(cpuid: usize, mmap_opt: Option<&[mem::phys::PMMapElement]>)
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
                mem::phys::pmm_init(mmap, kernel_start, kernel_end, None);
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
        // Todo: Need an event timer to implement sleep, etc.

        // Install the system call handlers
        arch::syscall_register(SyscallOpCode::TaskCtl,  syscall_task_control);
        arch::syscall_register(SyscallOpCode::ProcCtl,  syscall_proc_control);
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
            crate::mem::phys::pmm_num_free_frames());
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
        // Open the file (if exists)
        let Some(mnt) = MountPoint::from_path(path.as_str()) else {
            // Invalid mount point - try the next disk
            continue;
        };
        let fopen_ret = mnt.fopen(path.as_str());
        let drivers::storage::IOCompletion::Successful(hnd) = fopen_ret else {
            // File not found - try the next disk
            continue;
        };
        dbg!("  Found INIT program at {} on disk {} - Free frames: {}\n", 
                path, d, crate::mem::phys::pmm_num_free_frames());
        let Some(elf) = ELFBinary::from_file(mnt.clone(), hnd) else {
            // Invalid ELF file - try the next disk
            continue;
        };
        // elf.log_header();
        // klog!("Segments:\n");
        // for i in 0..elf.segments.len() {
        //     klog!("  {:X?}\n", elf.segments[i]);
        // }
        let pname = "shell.box".to_string();
        let Some(pid) = AddressSpace::spawn_from_elf(&elf, pname, path) else {
            // Failed to spawn the process from the ELF file - Panic
            panic!("Failed to spawn the INIT process from the ELF file!");
        };
        dbg!("  Launching the INIT process.. {}({}) - Free frames: {}\n",
                Task::name(), Task::current_tid(),
                crate::mem::phys::pmm_num_free_frames());
        Keyboard::clear_buffer();
        AddressSpace::launch_as_main(pid);
        // ELFBinary goes out of scope and releases closes the file        
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
    Open            = 2,
    Enum            = 3,
    Read            = 4,
    Write           = 5,
    Exec            = 6,
    Close           = 7,
    Max             = 8
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
    AddressSpace::launch(pid, func, farg);
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
    pub cmd_ptr: usize,// Input: Pointer to the cmd-line string in user-space
    pub cmd_len: usize,// Input: Length of the cmd-line string
    pub pid:    usize,  // Output
    pub m_tid:  usize,  // Output TID of the main task
}

pub struct ProcCtlResizeHeapArgs {
    pub delta:      isize,  // Input: Positive to expand, Negative to shrink
    pub heap_base:  usize,  // Output: New heap base (No change after the initial expansion)
    pub heap_size:  usize   // Output: New heap size
}

fn kuser_proc_launcher(pid: usize) {
    dbg!("kuser_proc_launcher: PID={}, TID={}\n", pid, Task::current_tid());
    // Box will be dropped here
    AddressSpace::launch_as_main(pid);
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
            copy_to_user(ret_ptr, 0);
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
        // Get the fops from the mount-point
        let Some(mnt) = MountPoint::from_path(path) else {
            // Invalid Mount Point
            copy_to_user(ret_ptr, 0);
            return;
        };
        // Open the ELF file
        let fopen_ret = mnt.fopen(path);
        let IOCompletion::Successful(hnd) = fopen_ret else {
            // Couldn't open the file
            copy_to_user(ret_ptr, 0);
            return;
        };
        // Parse the ELF file
        let Some(elf) = ELFBinary::from_file(mnt.clone(), hnd) else {
            // Invalid ELF file
            copy_to_user(ret_ptr, 0);
            return;
        };
        // Spawn a new process address space
        let Some(pid) = AddressSpace::spawn_from_elf(&elf,
                                pname.to_string(), cmd_line.to_string()) else {
            // Failed to spawn the process from the ELF file
            copy_to_user(ret_ptr, 0);
            return;
        };
        dbg!("  Launching user-space process.. {}({}) - Free frames: {}\n",
                Task::name(), Task::current_tid(),
                crate::mem::phys::pmm_num_free_frames());
        // Spawn the main task of the new process
        let tname = format!("P[{}].main", pid).to_string();
        info.m_tid = Task::spawn_named(kuser_proc_launcher, pid, tname);
        info.pid = pid;
        // Return the PID back to the caller
        copy_to_user(args, info);
        copy_to_user(ret_ptr, size_of::<ProcCtlSpawnArgs>());
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
            if let Some((new_base, new_size)) = AddressSpace::resize_heap(delta) {
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
// Virtual File System Control System Calls
//

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

fn syscall_open(path_ptr: usize, path_len: usize, _mode: usize, ret_ptr: usize) {
    let pathb;
    unsafe {
        pathb = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
    }
    let Ok(path) = str::from_utf8(pathb) else {
        // Invalid path string
        copy_to_user(ret_ptr, 0 as usize);
        return;
    };
    if let Some(mnt) = MountPoint::from_path(path) {
        let fopen_ret = mnt.fopen(path);      
        match fopen_ret {
            drivers::storage::IOCompletion::Successful(hnd)  => {
                // let fd = AddressSpace::add_file_object(pid, f);
                // klog!("OPEN syscall successful for {} hnd={}\n", path, hnd);
                let pid = Task::current_pid();
                let fd_obj = FileDescriptor {
                                fs_handle: hnd,
                                mount_name: mnt.name.clone(),
                                read_off: 0,
                                write_off: 0,
                };
                let fd = AddressSpace::add_fd(pid, fd_obj);
                if ret_ptr != 0 {
                    // For now just add 4 to the hnd to obtain an FD
                    copy_to_user(ret_ptr, fd);
                }
            },
            _                   => {
                dbg!("OPEN syscall failed for {} w\\ {:?}\n", path, fopen_ret);
                copy_to_user(ret_ptr, 0 as usize);
            }
        }
    } else {
        // Device not found
        // TODO need a way to pass different error codes
        copy_to_user(ret_ptr, 0 as usize);
    }
    // klog!("\nSyscall: OPEN({}, {}, {}) by PID:{} TID:{}\n{:?}", 
    //         path, path_len, mode, Task::current_pid(), Task::current_tid());
}

fn syscall_read(fd: usize, buf: usize, len: usize, ret_ptr: usize) {
    if fd == SyscallRsvdFDs::StandardIO as usize && len >= 1 {
        // Standard input - keyboard
        unsafe {
            // return the last keychar to the user-space
            let out = core::slice::from_raw_parts_mut(buf as *mut u8, len);
            out[0] = Keyboard::pop_ascii();
                
        }
        return;
    } else if fd == SyscallRsvdFDs::SystemResources as usize {
        // Resource Enumeration
        let out = unsafe {core::slice::from_raw_parts_mut(buf as *mut u8, len)};
        let ret_val = syscall_read_resources(out, 0);
        copy_to_user(ret_ptr, ret_val);
        return;
    }
    // Normal File Read
    let pid = Task::current_pid();
    if let Some(mut fd_obj) = AddressSpace::get_fd(pid, fd) {
        if let Some(mnt) = MountPoint::from_path(&fd_obj.mount_name) {
            unsafe {
                let out = core::slice::from_raw_parts_mut(buf as *mut u8, len);
                let ioc = mnt.fread(fd_obj.fs_handle, fd_obj.read_off, out);
                let ret_val;
                if let IOCompletion::Successful(len) = ioc {
                    fd_obj.read_off += len;
                    AddressSpace::update_fd(pid, fd, &fd_obj);
                    ret_val = len;
                } else {
                    ret_val = 0;
                }
                copy_to_user(ret_ptr, ret_val);
            }
        } else {
            // Invalid Mount Point
            dbg!("READ syscall failed due to invalid Mount Point {}\n", fd);
            copy_to_user(ret_ptr, 0 as usize);
        }
    } else {
        // Invalid FD
        dbg!("READ syscall failed due to invalid FD {}\n", fd);
        copy_to_user(ret_ptr, 0 as usize);
    }
}

fn syscall_enum(fd: usize, buf: usize, buf_len: usize, ret_ptr: usize) {
    if fd <= SyscallRsvdFDs::SystemResources as usize {
        //Enum not supported on special FDs!
        copy_to_user(ret_ptr, 0 as usize);
        return;
    }

    let pid = Task::current_pid();
    if let Some(fd_obj) = AddressSpace::get_fd(pid, fd) {
        if let Some(mnt) = MountPoint::from_path(&fd_obj.mount_name) {
            let mut out_vec: Vec<DirectoryEntry> = Vec::new();
            let ioc = mnt.fenum(fd_obj.fs_handle, &mut out_vec);
            if let IOCompletion::Successful(_cnt) = ioc {
                let mut serialized = String::new();
                for item in out_vec {
                    serialized += format!("{}, {}, 0x{:X}\n",
                                    item.name, item.size, item.flags).as_str();
                }
                let len = min(buf_len, serialized.len());
                unsafe {
                    let out = core::slice::from_raw_parts_mut(buf as *mut u8, len);
                    out[0..len].copy_from_slice(serialized.as_bytes());
                }
                copy_to_user(ret_ptr, len);
            } else {
                // Enum failed/not-supported
                copy_to_user(ret_ptr, 0 as usize);
            }
        } else {
            // Invalid Mount Point
            dbg!("ENUM syscall failed due to invalid Mount Point {}\n", fd);
            copy_to_user(ret_ptr, 0 as usize);
        }
    } else {
        // Invalid FD
        dbg!("ENUM syscall failed due to invalid FD {}\n", fd);
        copy_to_user(ret_ptr, 0 as usize);
    }    
}


fn syscall_write(fd: usize, buf: usize, len: usize, ret_ptr: usize) {
    if fd == SyscallRsvdFDs::StandardIO as usize {
        // Standard output - VGA console
        unsafe {
            kearly_console::print_str(
                        core::slice::from_raw_parts(buf as *const u8, len));
            
        }
        return;
    }
    // Normal File Write
    let pid = Task::current_pid();
    if let Some(mut fd_obj) = AddressSpace::get_fd(pid, fd) {
        if let Some(mnt) = MountPoint::from_path(&fd_obj.mount_name) {
            unsafe {
                let out = core::slice::from_raw_parts_mut(buf as *mut u8, len);
                let ioc = mnt.fwrite(fd_obj.fs_handle, fd_obj.write_off, out);
                let ret_val;
                if let IOCompletion::Successful(len) = ioc {
                    fd_obj.write_off += len;
                    AddressSpace::update_fd(pid, fd, &fd_obj);
                    ret_val = len;
                } else {
                    ret_val = 0;
                }
                copy_to_user(ret_ptr, ret_val);
            }
        } else {
            // Invalid Mount Point
            dbg!("WRITE syscall failed due to invalid Mount Point {}\n", fd);
            copy_to_user(ret_ptr, 0 as usize);
        }
    } else {
        // Invalid FD
        dbg!("WRITE syscall failed due to invalid FD {}\n", fd);
        copy_to_user(ret_ptr, 0 as usize);
    }
    
}

fn syscall_exec(fd: usize, cmd_ptr: usize, cmd_len: usize, fr_ptr: usize) {
    let pid = Task::current_pid();
    if fd == SyscallRsvdFDs::StandardIO as usize {
        // Exec the command on the shell
        if cmd_ptr == 1 && cmd_len == 0 {
            // Clear screen
            kearly_console::init();
        }
    } else if let Some(fd_obj) = AddressSpace::get_fd(pid, fd) {
        // Normal files/devices
        if let Some(mnt) = MountPoint::from_path(&fd_obj.mount_name) {
            let cmd_func;
            let mut ret_val: usize = 0;
            unsafe {
                cmd_func = (fr_ptr as *mut usize).read();
                let cmd_buf = core::slice::from_raw_parts_mut(cmd_ptr as *mut u8, cmd_len);

                dbg!("syscall_exec called on fd:{}, mnt:{}, func:{}\n",
                    fd, mnt.name, cmd_func
                    );

                let ioc = mnt.fexec(fd_obj.fs_handle, cmd_func, cmd_buf);
                if let IOCompletion::Successful(len) = ioc {
                    ret_val = len;
                }
                copy_to_user(fr_ptr, ret_val);
            }
        } else {
            // Invalid Mount Point
            dbg!("EXEC syscall failed due to invalid Mount Point {}\n", fd);
            copy_to_user(fr_ptr, 0 as usize);
        }
    } else {
        // Invalid FD
        dbg!("EXEC syscall failed due to invalid FD {}\n", fd);
        copy_to_user(fr_ptr, 0 as usize);
    }    
}

fn syscall_close(fd: usize, _: usize, _: usize, _: usize) {
    let pid = Task::current_pid();
    let mut closed = false;
    if let Some(fd_obj) = AddressSpace::get_fd(pid, fd) {
        if let Some(mnt) = MountPoint::from_path(&fd_obj.mount_name) {
            mnt.fclose(fd_obj.fs_handle);
            closed = true;
        }
    }
    if closed {
        AddressSpace::rem_file_object(pid, fd);
    }
}

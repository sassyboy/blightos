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
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use alloc::{format, str};
use alloc::string::String;
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
static USER_INIT_PID: AtomicUsize = AtomicUsize::new(0);

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
        Task::spawn_on_cpu(kinit_task, cpuid,String::from("kInit"));
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
fn kinit_task() {
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

    // Find the initial binary (disk%d.0:/blightos/shell.elf) to load
    for d in 0..ndisks {
        let path = format!("disk{}.0:/blightos/shell.elf", d);
        // Open the file (if exists)
        if let Some(mnt) = MountPoint::from_path(path.as_str()) {
            let fopen_ret = mnt.fopen(path.as_str());
            if let drivers::storage::IOCompletion::Successful(hnd) = fopen_ret {
                dbg!("  Found INIT program at {} on disk {} (hnd {})\
                        (Free frames: {})\n", 
                        path, d, hnd, crate::mem::phys::pmm_num_free_frames());
                if let Some(elf) = ELFBinary::from_file(mnt.clone(), hnd) {
                    // elf.log_header();
                    // klog!("Segments:\n");
                    // for i in 0..elf.segments.len() {
                    //     klog!("  {:X?}\n", elf.segments[i]);
                    // }
                    if let Some(pid) = AddressSpace::spawn_from_elf(&elf) {
                        USER_INIT_PID.store(pid, Ordering::Relaxed);
                        dbg!("  Launching {} (Free frames: {})\n", path,
                                    crate::mem::phys::pmm_num_free_frames());
                    }
                }
                // ELFBinary goes out of scope and releases closes the file
            }
        }
    }

    // Spawn the initial user-space process and convert this task into a
    // user-space task in the initial process!
    dbg!("  Switching to the INIT user-space process.. {}({}) \
            (Free frames: {})\n",
            Task::name(), Task::current_tid(),
            crate::mem::phys::pmm_num_free_frames());
    Keyboard::clear_buffer();
    AddressSpace::launch(USER_INIT_PID.load(Ordering::Relaxed));
}


///////
/// User-space interface
/// 

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

//
// Task Control System Call
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
pub struct UserTaskInfo{
    pub tid:        usize,
    pub pid:        usize,
    pub name:       [u8; 64]
}

fn syscall_task_control(opcode: usize, args: usize, ret_ptr: usize, _: usize) {
    if opcode == TaskControlOpCode::Exit as usize {
        // EXIT
        klog!("\nProgram ({} - MainTID:{}) Exited with status {}\n",
                                    Task::name(), Task::current_tid(), args);
        Task::exit();
    } else if opcode == TaskControlOpCode::Current as usize {
        // CURRENT
        let tinfo: *mut UserTaskInfo = args as *mut UserTaskInfo;
        let mut tname_out = [0 as u8; 64];
        let tname = Task::name();
        let tname_len = min(tname.len(), 63);
        tname_out[0..tname_len].copy_from_slice(&tname.as_bytes()[0..tname_len]);

        unsafe {
            tinfo.write(UserTaskInfo {
                tid: Task::current_tid(),
                pid: Task::current_pid(),
                name: tname_out
            });
            if ret_ptr != 0 {
                (ret_ptr as *mut usize).write(size_of::<UserTaskInfo>());
            }
        }
        
    } else if opcode == TaskControlOpCode::Spawn as usize {
        // SPAWN (Creates a task in the current process address space)
        klog!("\nTaskControlOpCode::Spawn syscall not implemented!\n");
    } else if opcode == TaskControlOpCode::Join as usize {
        klog!("\nTaskControlOpCode::Join syscall not implemented!\n");
    } else {
        klog!("\nInvalid TaskControlOpcode\n");
    }
    
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

fn syscall_open(path_ptr: usize, path_len: usize, _mode: usize, ret_ptr: usize) {
    let pathb;
    unsafe {
        pathb = core::slice::from_raw_parts(path_ptr as *const u8, path_len);
    }
    let path = str::from_utf8(pathb).unwrap();
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
                   unsafe { (ret_ptr as *mut usize).write(fd); }
                }
            },
            _                   => {
                dbg!("OPEN syscall failed for {} w\\ {:?}\n", path, fopen_ret);
                if ret_ptr != 0 {
                    unsafe { (ret_ptr as *mut usize).write(0); }
                }
            }
        }
    } else {
        // Device not found
        // TODO need a way to pass different error codes
        if ret_ptr != 0 {
            unsafe { (ret_ptr as *mut usize).write(0); }
        }
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
        unsafe {
            let out = core::slice::from_raw_parts_mut(buf as *mut u8, len);
            let ret_val = syscall_read_resources(out, 0);
            if ret_ptr != 0 {
                (ret_ptr as *mut usize).write(ret_val);
            }
        }
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
                if ret_ptr != 0 {
                    (ret_ptr as *mut usize).write(ret_val);
                }
            }
        } else {
            // Invalid Mount Point
            dbg!("READ syscall failed due to invalid Mount Point {}\n", fd);
            if ret_ptr != 0 {
                unsafe {(ret_ptr as *mut usize).write(0);}
            }
        }
    } else {
        // Invalid FD
        dbg!("READ syscall failed due to invalid FD {}\n", fd);
        if ret_ptr != 0 {
            unsafe {(ret_ptr as *mut usize).write(0);}
        }
    }
}

fn syscall_enum(fd: usize, buf: usize, buf_len: usize, ret_ptr: usize) {
    if fd <= SyscallRsvdFDs::SystemResources as usize {
        //Enum not supported on special FDs!
        unsafe {
            if ret_ptr != 0 {
                (ret_ptr as *mut usize).write(0);
            }
                
        }
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
                
                if ret_ptr != 0 {
                    unsafe {(ret_ptr as *mut usize).write(len);}
                }    
            } else {
                // Enum failed/not-supported
                if ret_ptr != 0 {
                    unsafe { (ret_ptr as *mut usize).write(0); }
                }
            }
        } else {
            // Invalid Mount Point
            dbg!("ENUM syscall failed due to invalid Mount Point {}\n", fd);
            if ret_ptr != 0 {
                unsafe {(ret_ptr as *mut usize).write(0);}
            }
        }
    } else {
        // Invalid FD
        dbg!("ENUM syscall failed due to invalid FD {}\n", fd);
        if ret_ptr != 0 {
            unsafe {(ret_ptr as *mut usize).write(0);}
        }
    }    
}


fn syscall_write(fd: usize, buf: usize, len: usize, _ret_ptr: usize) {
    if fd == SyscallRsvdFDs::StandardIO as usize {
        // Standard output - VGA console
        unsafe {
            kearly_console::print_str(
                        core::slice::from_raw_parts(buf as *const u8, len));
            
        }
    }
    
}

fn syscall_exec(fd: usize, cmd_ptr: usize, cmd_len: usize, fr_ptr: usize) {
    let pid = Task::current_pid();
    if let Some(fd_obj) = AddressSpace::get_fd(pid, fd) {
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
                if fr_ptr != 0 {
                    (fr_ptr as *mut usize).write(ret_val);
                }
            }
        } else {
            // Invalid Mount Point
            dbg!("EXEC syscall failed due to invalid Mount Point {}\n", fd);
            if fr_ptr != 0 {
                unsafe {(fr_ptr as *mut usize).write(0);}
            }
        }
    } else {
        // Invalid FD
        dbg!("EXEC syscall failed due to invalid FD {}\n", fd);
        if fr_ptr != 0 {
            unsafe {(fr_ptr as *mut usize).write(0);}
        }
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

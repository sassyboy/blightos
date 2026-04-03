//
// BlightOS Kernel
//
// Process Address Space Management
//
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::arch::MMUMapping;
use crate::{arch, sched::Task};
use crate::mem::phys::*;
use crate::ProcCtlGetInfoArgs;
use crate::fs::File;
use crate::util::*;
use crate::*;

#[cfg(feature="debug_virt")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[VIRT] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}
#[cfg(not(feature="debug_virt"))]
macro_rules! dbg{
    ($($arg:tt)*) => { };
}


static PROCESSES: Spinlock<BTreeMap<usize, AddressSpace>> = 
        Spinlock::new(BTreeMap::new());

static NEXT_PID: AtomicUsize = AtomicUsize::new(1);

static DEFAULT_USTACK_INIT_PAGES: AtomicUsize = AtomicUsize::new(4);
static DEFAULT_USTACK_MAX_PAGES:  AtomicUsize = AtomicUsize::new(4096);
//
// Encapsulates the information about a user-space task in the context of a
// process address space. This is complementary to the Task structure in sched,
// which is more about the scheduling and execution context of a task.
// All address fields are virtual, unless specified.
//
struct UserTask {
    tid:                usize, // TID of the task in the kernel scheduler
    user_sp:            usize, // Current user-space stack pointer of the task
    // User-space stack range (grows down)
    ustack_base:        usize, // The largest address of the user stack
    ustack_pages:       usize, // # of pages currently allocated for the user stack
    ustack_max_pages:   usize,
}
impl UserTask {
    // The lowest address currently mapped as the part of the user stack.
    pub fn stack_top(&self) -> usize {
        self.ustack_base - (self.ustack_pages * MMUMapping::PAGE_SIZE) + 1
    }
}
// Virtual Address Space Layout:
// Kernel (mapped in all processes):
// 0                          to MMUMapping::MIN_VIRTUAL -1 (8 GB)
// User-space (mapped separately for each process):
// MMUMapping::MIN_VIRTUAL GB to MMUMapping::MAX_VIRTUAL
//
// User-space layout:
// [Program image]   : img_base, i.e., MMUMapping::MIN_VIRTUAL
// [Heap (grows up)] : heap_base = img_base + img_pages 
// ...... Gap ...... : heap_base + heap_pages to stack_top
//                   <-- stack_top, where the next (new) task's stack starts
// [2nd task's stack]: MMUMapping::MAX_VIRTUA - MAX_USTACK_SIZE
// [1st task's stack]: MMUMapping::MAX_VIRTUA (grows down)
pub struct AddressSpace {
    pid:            usize,
    name:           String, // Name of the process, e.g., shell.box
    cmd_line:        String,// The command string used to launch this process,
                            // e.g., "disk0.0:/blightos/shell.box arg1 arg2"
                            // The first part is always the absolute path of the
                            // binary used to launch this process
    // Architecture-dependent address-space object
    vmap:           MMUMapping,
    // Program image range in memory
    img_base_pys:   usize,
    img_pages:      usize,
    ep_vaddr:       usize, // Virtual - Entry point of the program image
    // Heap range (virtual)
    heap_base:      usize,
    heap_pages:     usize,
    // User-stack information
    stack_top:      usize, // Virtual - The next task's stack starts from here
    // Tasks belonging to this process
    // The first task is the main/root task, whose termination causes the
    // process (and other child tasks)to be freed.
    children:       Vec<UserTask>,
    // File Descriptor -> File objects
    files:          Vec<File>,
    // TODOs: 
    //   Heap start/end, brk pointer, etc.
    //   Number of pages allocated for the program image, stack, heap, etc.
    //   Process VFS entry to report various status info (memory usage, open
    //     files, etc.)
    //   FORK and EXEC system call support
    //   SIGNALS and IPC
    //   Shared memory management (between processes and with the kernel)
}

impl AddressSpace {
    pub const fn new() -> Self {
        Self {
            pid:            0,
            name:           String::new(),
            cmd_line:       String::new(),
            vmap:           MMUMapping::new(),
            img_base_pys:   0,
            img_pages:      0,
            ep_vaddr:       0,
            heap_pages:     0,
            heap_base:      0,
            stack_top:      MMUMapping::MAX_VIRTUAL as usize,
            children:       Vec::new(),
            files:          Vec::new(),
            
        }
    }

    pub fn get_main_tid(pid: usize) -> Option<usize> {
        let proc_map = PROCESSES.lock();
        let proc = proc_map.get(&pid);
        match proc {
            Some(p) => {
                if p.children.len() > 0 {
                    return Some(p.children[0].tid);
                }
                None
            },
            None    => None
        }
    }

    pub fn get_process_info(pid: usize) -> Option<ProcCtlGetInfoArgs> {
        let proc_map = PROCESSES.lock();
        let proc = proc_map.get(&pid);
        
        match proc {
            Some(p) => {
                let mut info = ProcCtlGetInfoArgs {
                    pid:                p.pid,
                    name:               [0 as u8; 64],
                    cmd_line:           [0 as u8; 1024],
                    main_tid:           p.children[0].tid,
                    task_count:         p.children.len(),
                    fd_count:           p.files.len(),
                    img_base:           MMUMapping::MIN_VIRTUAL as usize,
                    img_size:           p.img_pages * PHY_FRAME_SIZE,
                    heap_base:          p.heap_base,
                    heap_size:          p.heap_pages * PHY_FRAME_SIZE,
                    stack_top:          p.stack_top,
                    total_mem_usage:    (p.vmap.mapped_pages_count() + 
                                        p.vmap.tlb_page_count()) * 
                                                MMUMapping::PAGE_SIZE,
                    meta_mem_usage:     (p.vmap.tlb_page_count()) * 
                                                MMUMapping::PAGE_SIZE,
                };
                // Copy the process name
                let name_bytes = p.name.as_bytes();
                let name_len = if name_bytes.len() < 64 {
                    name_bytes.len()
                } else {
                    63
                };
                info.name[..name_len].copy_from_slice(&name_bytes[..name_len]);
                // Copy the process command line
                let cmd_bytes = p.cmd_line.as_bytes();
                let cmd_len = if cmd_bytes.len() < 1024 {
                    cmd_bytes.len()
                } else {
                    1023
                };
                info.cmd_line[..cmd_len].copy_from_slice(&cmd_bytes[..cmd_len]);
                Some(info)
            },
            None    => None
        }
    }

    // Program image is already loaded and marked in the physical memory.
    // Just initialize the Process object fields
    // This can be called from any thread context as it doesn't perform any
    // mapping, stack allocation.
    // Adds the process to the process pool and returns the PID of the new
    // process address space
    pub fn spawn(img_phy_base: usize, img_mem_size: usize, ep_vaddr: usize, 
                        name: String, cmd_line: String) -> Result<usize, Error>
    {
        let mut proc_map = PROCESSES.lock();
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        proc_map.insert(pid, Self::new());
        let proc = proc_map.get_mut(&pid);
        match proc {
            Some(p) => {
                p.name          = name;
                p.cmd_line      = cmd_line;
                p.ep_vaddr      = ep_vaddr;
                p.img_base_pys  = img_phy_base;
                p.img_pages     = div_round_up!(img_mem_size, PHY_FRAME_SIZE);
                p.heap_base     = MMUMapping::MIN_VIRTUAL as usize + 
                                    (p.img_pages * PHY_FRAME_SIZE);
                p.heap_pages    = 0;
                p.pid           = pid;
            }
            None    => return Err(error!(ErrorCode::InvalidPID))
        }
        Ok(pid)
    }

    //
    // Loads a program image in the memory from a given ELF image and calls
    // spawn, which returns the PID of the new process address space.
    //
    pub fn spawn_from_elf(elf: &mut ELFBinary, pname: String, cmd_line: String) 
                                                    -> Result<usize, Error> {
        // Calculate the total memory size needed for the program image from the
        // ELF segments
        let mut mem_sz = 0;
        for seg in elf.segments.iter() {
            if seg.p_type == ELFSegment::P_TYPE_LOAD {
                mem_sz += seg.p_memsz;
            }
        }
        let frame_count = div_round_up!(mem_sz, PHY_FRAME_SIZE);
        let mut xfer_len = 0;
        // TODO - Allocate at a 4KB granularity instead of continuously
        let alloc = palloc_continuous(frame_count);
        if let Some(phys_base) = alloc {
            dbg!("spawn_from_elf: Size: {} bytes, needs {} frames - \
                    start: {:X}, end: {:X}\n",
                    mem_sz, frame_count, phys_base,
                    phys_base + (frame_count * PHY_FRAME_SIZE) - 1);
            // Go over the sections and copy them over
            for i in 0..elf.segments.len() {
                if elf.segments[i].p_type == ELFSegment::P_TYPE_LOAD {
                    xfer_len += 
                    elf.load_segment(i, phys_base + elf.segments[i].p_offset)?;
                }
            }
        } else {
            klog!("spawn_from_elf - Couldn't allocate {} frames", frame_count);
            return Err(error!(ErrorCode::OutOfMemory));
        }
        if xfer_len != mem_sz {
            klog!("spawn_from_elf transferred {} bytes < {}\n", xfer_len,
                        mem_sz);
            // Release the memory
            pfree_continuous(alloc.unwrap(), frame_count);
        }
        return Self::spawn(alloc.unwrap(), mem_sz, elf.elf_entry, pname, cmd_line);
    }

    /// This is called from the context of the kernel task that is going to
    /// be converted to the MAIN user-space task of process identified by `pid`.
    /// The process must be spawned, but not yet launched, meaning that it has
    /// its program image loaded into physical memory, butdoesn't
    /// have any virtual memory mappings, and any children (tasks) yet.
    pub fn launch_as_main(pid: usize) -> bool {
        if pid == 0 {
            return false;
        }
        let ep_vaddr;
        let usr_stk_vptr;
        let adp_priv;
        {
            let mut proc_map = PROCESSES.lock();
            let proc = proc_map.get_mut(&pid);
            match proc {
            Some(p) => {
                // Map the initial user-space addresses
                if p.img_pages < 1 {
                    return false;
                }
                if p.children.len() > 0 {
                    return false; // Already launched
                }
                // Initialize the Arch-dependent PAS object (maps the kernel)
                p.vmap.init();
                // Map the program image at the start of the user-space VAS
                p.vmap.map_pages(
                    MMUMapping::MIN_VIRTUAL as usize,
                    p.img_base_pys, p.img_pages,
                    false, true, true, arch::MmuCachingPolicy::WriteBack
                );
                // Attach the main/root task to this process. This will allocate
                // the user-stack for this task and set the stack_top accordingly.
                 let main_task = UserTask {
                    tid:            Task::current_tid(),
                    user_sp:        0, // Set by attach_task
                    ustack_base:    0, // Set by attach_task
                    ustack_pages:   0, // Set by attach_task
                    ustack_max_pages: DEFAULT_USTACK_MAX_PAGES
                                            .load(Ordering::Relaxed),
                };
                if !p.attach_task(main_task) || p.children.len() != 1 {
                    return false;
                }
                ep_vaddr        = p.ep_vaddr;
                adp_priv        = p.vmap.copy_priv_data();
                usr_stk_vptr    = p.children[0].user_sp;
            }
            None    => {
                return false;
            }
            }
        } // ProcessPool unlocked
        MMUMapping::move_to_userspace(adp_priv, ep_vaddr, 0,
                                            usr_stk_vptr, Self::task_exited);
        panic!("Must have been unreachable!");
    }

    /// This is called from the context of the kernel task that is going to
    /// be converted to a user-space task of the process identified by `pid`.
    /// The process must be already spawned and launched (main task attached)
    /// before this is called.
    pub fn launch(pid: usize, user_func: fn(usize), func_arg: usize) {
        if pid < 1 {
            return;
        }
        let usr_stk_vptr;
        let adp_priv;
        {
            let mut proc_map = PROCESSES.lock();
            let proc = proc_map.get_mut(&pid);
            match proc {
            Some(p) => {
                if p.children.len() < 1 {
                    return; // Process not launched yet
                }
                let new_task = UserTask {
                    tid:            Task::current_tid(),
                    user_sp:        0, // Set by attach_task
                    ustack_base:    0, // Set by attach_task
                    ustack_pages:   0, // Set by attach_task
                    ustack_max_pages: DEFAULT_USTACK_MAX_PAGES
                                            .load(Ordering::Relaxed),
                };
                if !p.attach_task(new_task) {
                    return;
                }
                adp_priv        = p.vmap.copy_priv_data();
                usr_stk_vptr    = p.children.last().unwrap().user_sp;
            },
            None    => {
                return;
            }
            }
        } // ProcessPool unlocked
        MMUMapping::move_to_userspace(adp_priv, user_func as usize, 
                                    func_arg, usr_stk_vptr, Self::task_exited);
        panic!("Must have been unreachable!");
    }

    // Attaches a task to this process address space by setting up the user-space
    // stack for this task, mapping it in the VAS, updating the Task structure
    // to reflect the PID, and adding the task to the children list of this
    // process.
    fn attach_task(&mut self, mut utask: UserTask) -> bool {
        if !Task::exists(utask.tid) {
            return false;
        }
        // Allocate a user-stack for this task below the current stack_top
        if self.stack_top - (utask.ustack_max_pages * MMUMapping::PAGE_SIZE) <= 
                self.heap_base + (self.heap_pages * MMUMapping::PAGE_SIZE) {
            return false; // No more space for stacks!
        }
        utask.user_sp       = self.stack_top + 1;
        utask.ustack_base   = self.stack_top;
        utask.ustack_pages  = DEFAULT_USTACK_INIT_PAGES.load(Ordering::Relaxed);
        let ustack_phys = palloc_continuous(utask.ustack_pages)
                                                    .expect("Out of memory");
        self.vmap.map_pages(
            utask.stack_top(), ustack_phys, utask.ustack_pages,
            false, true, false, arch::MmuCachingPolicy::WriteBack
        );
        dbg!("Attached task TID:{} to PID:{} - Stack: {:X} - {:X}, SP: {:X}\n",
            utask.tid, self.pid,
            utask.stack_top(),
            utask.ustack_base,
            utask.user_sp
        );
        // self.vmap.log_mapping(utask.user_ep);
        // Update the Task structure to reflect the PID
        Task::set_pid(self.pid);
        // Update the stack_top for the next task
        self.stack_top -= utask.ustack_max_pages * MMUMapping::PAGE_SIZE;
        dbg!("Updated process stack_top to {:X}\n", self.stack_top);
        // Insert the task in the children list of this process
        self.children.push(utask);
        true
    }
    //
    // Loads a process into the memory from the currenly running process
    // and then calls spawn and returns the PID of the new process address space
    pub fn fork() -> usize{
        panic!("Not supported!");
    }

    // Called by the drop handler of a task belonging to this PID after the
    // task strucutre is cleaned up, in the context of the CPU worker task.
    pub fn task_dropped(pid: usize, tid: usize) {
        if pid == 0 {
            return; // Not a valid process!
        }
        // Only clean up the process when the root task is dropped.
        // TODO account for terminating non-root tasks
        let mut proc_map = PROCESSES.lock();
        let proc_opt = proc_map.get_mut(&pid);
        match proc_opt {
            Some(p) => {
                if p.children[0].tid == tid {
                    proc_map.remove(&pid);
                }
            }
            None    => {}
        }
    }

    pub fn handle_page_fault(fault_addr: usize) -> bool {
        let kernel_text_start: usize;
        let kernel_text_end: usize;
        unsafe{
            kernel_text_start = &_KERNEL_TEXT_START as *const usize as usize;
            kernel_text_end = &_KERNEL_TEXT_END as *const usize as usize;
        }
        dbg!("Page fault at address {:X} in process {}, {:X} < KERNEL < {:X}\n",
            fault_addr, Task::current_pid(), kernel_text_start, kernel_text_end);
        if fault_addr >= kernel_text_start && fault_addr < kernel_text_end {
            // The user code tried to return the the exit handler MMU put on
            // its stack (via MMUMapping::move_to_userspace), but it page faults
            // because AddressSpace::task_exited is kernel code ;)
            // So we catch the page-fault here and call it here in the kernel.
            Self::task_exited();
            return true;
        }
        // TODO: Handle stack growth and heap growth page faults here
        false
    }

    fn task_exited() {
        dbg!("A user-space task exited! - TID: {}, PID: {}\n",
            Task::current_tid(), Task::current_pid());
         // Drop the current task, which will trigger the drop handler to clean
         // up the process if this is the root task.
         Task::exit();
    }
    //
    // User Memory Management (Heap, Stack, SHM, etc.)
    //

    /// Resizes the heap of the currently running process by the given delta 
    /// (positive to expand, negative to shrink).
    /// Returns the new heap base and size on success, or None on failure.
    pub fn resize_heap(delta: isize) -> Option<(usize, usize)> {
        if delta == 0 {
            // Return the current heap base and size without resizing
            let proc_map = PROCESSES.lock();
            let proc = proc_map.get(&Task::current_pid()).expect("Process not found!");
            return Some((proc.heap_base, proc.heap_pages * PHY_FRAME_SIZE));
        } else if delta > 0 {
            // Expand the heap
            let more_pages = div_round_up!(delta as usize, PHY_FRAME_SIZE);
            let mut proc_map = PROCESSES.lock();
            let proc = proc_map.get_mut(&Task::current_pid()).expect("Process not found!");
            // Check if there is enough space to expand the heap without colliding with the stack
            if proc.heap_base + ((proc.heap_pages + more_pages) * PHY_FRAME_SIZE) >= proc.stack_top {
                return None; // Not enough space to expand the heap
            }
            // Allocate additional pages for the heap
            let new_heap_pages = proc.heap_pages + more_pages;
            let new_heap_size = new_heap_pages * PHY_FRAME_SIZE;
            let alloc = palloc_continuous(more_pages);
            if let Some(phys_base) = alloc {
                // Map the new heap pages in the process's address space
                proc.vmap.map_pages(
                    proc.heap_base + (proc.heap_pages * PHY_FRAME_SIZE),
                    phys_base, more_pages,
                    false, true, true, arch::MmuCachingPolicy::WriteBack
                );
                // Update the process's heap information
                proc.heap_pages = new_heap_pages;
                return Some((proc.heap_base, new_heap_size));
            } else {
                return None; // Failed to allocate physical memory for the heap expansion
            }
        } else {
            // Shrink the heap
            let shrink_pages = div_round_up!((-delta) as usize, PHY_FRAME_SIZE);
            let mut proc_map = PROCESSES.lock();
            let proc = proc_map.get_mut(&Task::current_pid()).expect("Process not found!");
            if shrink_pages > proc.heap_pages {
                return None; // Cannot shrink more than the current heap size
            }
            // Unmap and free the pages being removed from the heap
            for i in 0..shrink_pages {
                let page_vaddr = proc.heap_base + ((proc.heap_pages - 1 - i) * PHY_FRAME_SIZE);
                if let Some(page_paddr) = proc.vmap.unmap_page(page_vaddr) {
                    pfree(page_paddr);
                }
            }
            // Update the process's heap information
            proc.heap_pages -= shrink_pages;
            return Some((proc.heap_base, proc.heap_pages * PHY_FRAME_SIZE));
        }
    }

    //
    // File descriptor management
    //
    /// Adds a file object to the currently running process and returns the file
    /// descriptor (FD) of the new file object
    pub fn add_file(pid: usize, fobj: File) -> usize {
        let mut proc_map = PROCESSES.lock();
        let proc = proc_map.get_mut(&pid).expect("Process not found!");
        proc.files.push(fobj);
        proc.files.len() - 1 + crate::SyscallRsvdFDs::Max as usize
    }

    /// Returns a clone of the file object corresponding to the given file
    /// descriptor (FD) without reopening the file.
    pub fn get_file(pid: usize, fd: usize) -> Result<File, Error> {
        let fd_index = fd - crate::SyscallRsvdFDs::Max as usize;
        let mut proc_map = PROCESSES.lock();
        match proc_map.get_mut(&pid) {
            Some(proc) => {
                if fd_index < proc.files.len() {
                    return Ok(proc.files[fd_index].clone());
                }
                Err(error!(ErrorCode::InvalidFD))
            },
            None => Err(error!(ErrorCode::InvalidPID))
        }
    }

    pub fn remove_file(pid: usize, fd: usize) {
        let fd_index = fd - crate::SyscallRsvdFDs::Max as usize;
        let mut proc_map = PROCESSES.lock();
        match proc_map.get_mut(&pid) {
            Some(proc) => {
                if fd_index < proc.files.len() {
                    proc.files.remove(fd_index);
                }
            },
            None => {}
        }
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        // self.vmap.drop will be called after this drop(), which releases the
        // paging structs and any physical frames (program image, stack, etc.)
        // pointed to by those structs
        // TODO: Close any open files that the user failed to close
    }
}

unsafe extern "C" {
    unsafe static _KERNEL_TEXT_START: usize;
    unsafe static _KERNEL_TEXT_END: usize;
}
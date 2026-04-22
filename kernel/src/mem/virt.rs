//
// BlightOS Kernel
//
// Process Address Space Management
//
// TODOs: 
//   Process VFS entry to report various status info (memory usage, open
//     files, etc.)
//   FORK and EXEC system call support
//   SIGNALS and IPC
//   Shared memory management (between processes and with the kernel)

use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use crate::arch::MMUMapping;
use crate::sched::Task;
use crate::mem::{phys::PhysMem, MemoryType};
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
static DEFAULT_USTACK_EXPAND_PAGES: AtomicUsize = AtomicUsize::new(4);
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
    /// The smallest address currently mapped as the part of the user stack.
    pub fn stack_top(&self) -> usize {
        self.ustack_base - (self.ustack_pages * MMUMapping::PAGE_SIZE) + 1
    }
}
// Virtual Address Space Layout:
// Kernel (mapped in all processes):
// 0                          to MMUMapping::MIN_VIRTUAL -1 (8 GB)
//
// User-space (mapped separately for each process):
// MMUMapping::MIN_VIRTUAL GB to MMUMapping::MAX_VIRTUAL
//
// User-space layout:
// [Program image]   : img_base, i.e., MMUMapping::MIN_VIRTUAL
// [Heap (grows up)] : heap_base = img_base + img_pages 
// ...... Gap ...... : heap_base + heap_pages to stack_top
//                   <-- stack_top, where the next (new) task's stack starts
// [2nd task's stack]: MMUMapping::MAX_USTACK_VIRTUAL - MAX_USTACK_SIZE
// [1st task's stack]: MMUMapping::MAX_USTACK_VIRTUAL (grows toward lower addrs)
// [dynamic map pool]: MMUMapping::MIN_USPOOL_VIRTUAL to MAX_VIRTUAL
pub struct AddressSpace {
    pid:            usize,
    name:           String, // Name of the process, e.g., shell.box
    cmd_line:       String, // The command string used to launch this process,
                            // e.g., "disk0.0:/blightos/shell.box arg1 arg2"
                            // The first part is always the absolute path of the
                            // binary used to launch this process
    // Architecture-dependent address-space object
    vmap:           MMUMapping,
    // Program image range in memory starts from MMUMapping::MIN_VIRTUAL_USER
    img_pages:      usize,      // Number of pages in the program image
    ep_vaddr:       usize,      // Virtual - Entry point of the program image
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
}

impl AddressSpace {
    //
    // Process Management methods
    // 
    pub const fn new() -> Self {
        Self {
            pid:            0,
            name:           String::new(),
            cmd_line:       String::new(),
            vmap:           MMUMapping::new(),
            img_pages:      0,
            ep_vaddr:       0,
            heap_pages:     0,
            heap_base:      0,
            stack_top:      MMUMapping::MAX_USTACK_VIRTUAL as usize,
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
                    img_base:           MMUMapping::MIN_VIRTUAL_USER as usize,
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

    /// Adds a new process to the process pool, initializes a new address space,
    /// and returns the PID of the new process.
    /// This can be called from any context as it doesn't perform any
    /// process-specific mapping, stack allocation, etc.
    pub fn spawn(proc_name: String, cmd_line: String) -> Result<usize, Error>
    {
        let mut proc_map = PROCESSES.lock();
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        proc_map.insert(pid, Self::new());
        let proc = proc_map.get_mut(&pid);
        match proc {
            Some(p) => {
                p.name          = proc_name;
                p.cmd_line      = cmd_line;
                p.heap_pages    = 0;
                p.pid           = pid;
                p.vmap.init();
            }
            None    => return Err(error!(ErrorCode::InvalidPID))
        }
        Ok(pid)
    }

    /// Loads a program image in the memory from a given ELF image, and converts
    /// the caller task into the main task of the process pid, and jumps to the
    /// entry point of the program image to start executing the user-space code.
    /// 
    /// This must be called from the context of the kernel task that
    pub fn launch_elf(pid: usize, mut elf: ELFBinary) -> Result<(), Error> {
        let mut proc_map = PROCESSES.lock();
        let proc;
        match proc_map.get_mut(&pid) {
            Some(p) => {
                proc = p;
            },
            None    => return Err(error!(ErrorCode::InvalidPID))
        }
        if proc.children.len() > 0 {
            // Process already launched with a main task
            klog!("launch_elf failed: PID {} already has a main task!\n", pid);
            return Err(error!(ErrorCode::InvalidOp));
        }
                
        // Calculate the total memory size needed for the program image from the
        // ELF segments
        let mut mem_sz = 0;
        for seg in elf.segments.iter() {
            if seg.p_type == ELFSegment::P_TYPE_LOAD {
                mem_sz += seg.p_memsz;
            }
        }
        let frame_count = div_round_up!(mem_sz, PHY_FRAME_SIZE);
        if frame_count < 1 {
            klog!("launch_elf failed: No loadable segments in the ELF image!\n");
            return Err(error!(ErrorCode::InvalidFormat));
        }
        // Allocate physical frames for the program image
        let mut frames = Vec::<usize>::with_capacity(frame_count);
        for _i in 0..frame_count {
            frames.push(0); // Initialize to zero
        }
        let _ = PhysMem::alloc_high_frames(frames.as_mut_slice())?;
        dbg!("launch_elf: Size: {} bytes - allocated {} frames\n",
                mem_sz, frame_count);
    
        // Switch to the virtual address space of the process
        proc.vmap.enter();
        proc.ep_vaddr      = elf.elf_entry;
        proc.img_pages     = div_round_up!(frame_count, PHY_FRAME_SIZE);
        proc.heap_base     = MMUMapping::MIN_VIRTUAL_USER as usize + 
                                    (frame_count * PHY_FRAME_SIZE);
        
        // Map the program image at the start of the user-space VAS as RWX
        let virt_base = MMUMapping::MIN_VIRTUAL_USER as usize;
        let frm_slc = frames.as_slice();
        proc.vmap.map_pages(virt_base, frm_slc, true, true, MemoryType::Normal);

        // Copy the ELF segments to the program image
        let virt_base = MMUMapping::MIN_VIRTUAL_USER as usize;
        let mut xfer_len = 0;
        for i in 0..elf.segments.len() {
            if elf.segments[i].p_type == ELFSegment::P_TYPE_LOAD {
                let xfer_res = elf.load_segment(i, virt_base +
                                                    elf.segments[i].p_offset);
                match xfer_res {
                    Ok(len) => xfer_len += len,
                    Err(e) => {
                        klog!("launch_elf failed to load segment {}: {:?}\n",
                                i, e);
                        // Release the memory
                        PhysMem::free_frames(frames.as_slice());
                        return Err(e);
                    }
                }
            }
        }
        if xfer_len != mem_sz {
            klog!("launch_elf copied {} bytes < {}\n", xfer_len, mem_sz);
            // Release the memory
            PhysMem::free_frames(frames.as_slice());
            return Err(error!(ErrorCode::IOError));
        }
        // Drop the ELF object
        drop(elf);

        // Attach the main/root task to this process. This will allocate
        // the user stack for this task and set the stack_top accordingly.
        let main_task = UserTask {
            tid:            Task::current_tid(),
            user_sp:        0, // Set by attach_task
            ustack_base:    0, // Set by attach_task
            ustack_pages:   0, // Set by attach_task
            ustack_max_pages: DEFAULT_USTACK_MAX_PAGES.load(Ordering::Relaxed),
        };
        proc.attach_task(main_task)?;
        if proc.children.len() != 1 {
            klog!("launch_elf failed to add the main task to the process!\n");
            // Release the memory
            PhysMem::free_frames(frames.as_slice());
            return Err(error!(ErrorCode::Other));
        }

        let ep_vaddr = proc.ep_vaddr;
        let mmu_priv = proc.vmap.copy_priv_data();
        let usr_stk  = proc.children[0].user_sp;
        drop(proc_map); // unlock ProcessPool

        // Jump to the entry point
        MMUMapping::move_to_userspace(mmu_priv, ep_vaddr, 0, usr_stk,
                                                            Self::task_exited);
    }
    
    /// Loads a process into the memory from the currenly running process
    /// and then calls spawn and returns the PID of the new process address space
    pub fn fork() -> usize{
        panic!("Not supported!");
    }

    //
    // User-space Task Management methods
    //

    /// This is called from the context of the kernel task that is going to
    /// be converted to a user-space task of the process identified by `pid`.
    /// The process must be already spawned and launched (main task attached)
    /// before this is called.
    pub fn move_to_process(pid: usize, user_func: fn(usize), func_arg: usize)
                                                        -> Result<(), Error> {
        if pid < 1 {
            return Err(error!(ErrorCode::InvalidPID));
        }
        let usr_stk_vptr;
        let adp_priv;
        {
            let mut proc_map = PROCESSES.lock();
            let Some(proc) = proc_map.get_mut(&pid) else {
                return Err(error!(ErrorCode::InvalidPID));
            };
            if proc.children.len() < 1 {
                // Process not launched yet
                return Err(error!(ErrorCode::InvalidArgument));
            }
            let new_task = UserTask {
                tid:            Task::current_tid(),
                user_sp:        0, // Set by attach_task
                ustack_base:    0, // Set by attach_task
                ustack_pages:   0, // Set by attach_task
                ustack_max_pages: DEFAULT_USTACK_MAX_PAGES
                                            .load(Ordering::Relaxed),
            };
            proc.attach_task(new_task)?;
            adp_priv        = proc.vmap.copy_priv_data();
            usr_stk_vptr    = proc.children.last().unwrap().user_sp;
        } // ProcessPool unlocked
        MMUMapping::move_to_userspace(adp_priv, user_func as usize, 
                                    func_arg, usr_stk_vptr, Self::task_exited);
    }

    // Attaches a task to this process address space by setting up the user-space
    // stack for this task, mapping it in the VAS, updating the Task structure
    // to reflect the PID, and adding the task to the children list of this
    // process.
    fn attach_task(&mut self, mut utask: UserTask) -> Result<(), Error> {
        if !Task::exists(utask.tid) {
            return Err(error!(ErrorCode::InvalidTID));
        }
        // Allocate a user-stack for this task below the current stack_top
        if self.stack_top - (utask.ustack_max_pages * MMUMapping::PAGE_SIZE) <= 
                self.heap_base + (self.heap_pages * MMUMapping::PAGE_SIZE) {
            return Err(error!(ErrorCode::OutOfMemory));
        }
        utask.user_sp       = self.stack_top + 1;
        utask.ustack_base   = self.stack_top;
        utask.ustack_pages  = DEFAULT_USTACK_INIT_PAGES.load(Ordering::Relaxed);
        
        // Allocate physical frames for the user stack
        let mut frames = Vec::<usize>::with_capacity(utask.ustack_pages);
        for _i in 0..utask.ustack_pages {
            frames.push(0); // Initialize to zero
        }
        let _ = PhysMem::alloc_high_frames(frames.as_mut_slice())?;
        dbg!("attach_task: Stack Size: {} frames\n", utask.ustack_pages);
        // Map the stack image at the start of the task's stack range as RW
        let virt_base = utask.stack_top();
        let frms = frames.as_slice();
        self.vmap.map_pages(virt_base, frms, true, false, MemoryType::Normal);
        
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
        Ok(())
    }

    // Called by the drop handler of a task belonging to this PID after the
    // task strucutre is cleaned up, in the context of the CPU worker task.
    pub fn task_dropped(pid: usize, tid: usize) {
        if pid == 0 {
            return; // Not a valid process!
        }
        // Only clean up the process when the root task is dropped.
        let mut proc_map = PROCESSES.lock();
        if let Some(p) =  proc_map.get_mut(&pid) {
            if p.children[0].tid == tid {
                // Drop the whole process as this was the main task
                proc_map.remove(&pid);
            } else {
                // Just remove the user task from the children list.
                // Should release the private user-space resources of the
                // task here as the drop handler can lock the proc_map
                if let Some(ut) = p.children.iter().find(|t| t.tid == tid) {
                    // Release the task's stack
                    let st_vbase = ut.stack_top();
                    for i in 0..ut.ustack_pages {
                        let vaddr = st_vbase + (i * MMUMapping::PAGE_SIZE);
                        if let Some(paddr) = p.vmap.unmap_page(vaddr) {
                            PhysMem::free(paddr);
                        }
                    }
                }
                p.children.retain(|t| t.tid != tid);
            }
        }
    }

    fn task_exited() {
        dbg!("A user-space task exited! - TID: {}, PID: {}\n",
            Task::current_tid(), Task::current_pid());
         // Drop the current task, which will trigger the drop handler to clean
         // up the process if this is the root task.
         Task::exit();
    }

    //
    // User Memory Management methods (Heap, Stack, SHM, etc.)
    //

    /// Resizes the heap of the currently running process by the given delta 
    /// (positive to expand, negative to shrink).
    /// Returns the new heap base and size on success, or None on failure.
    pub fn resize_heap(delta: isize) -> Result<(usize, usize), Error> {
        // Find the current proccess address space
        let mut proc_map = PROCESSES.lock();
        let Some(proc) = proc_map.get_mut(&Task::current_pid()) else {
            return Err(error!(ErrorCode::InvalidPID));
        };
        if delta == 0 {
            // Return the current heap base and size without resizing
            return Ok((proc.heap_base, proc.heap_pages * PHY_FRAME_SIZE));
        } else if delta > 0 {
            // Expand the heap
            let more_pages = div_round_up!(delta as usize, PHY_FRAME_SIZE);
            let new_heap_pages = proc.heap_pages + more_pages;
            let new_heap_size = new_heap_pages * PHY_FRAME_SIZE;
            // Check if there is enough space to expand the heap without
            // colliding with the stack
            if proc.heap_base + new_heap_size >= proc.stack_top {
                // Not enough space to expand the heap
                return Err(error!(ErrorCode::OutOfMemory)); 
            }
            // Allocate additional physical frames for the heap
            let mut frames = Vec::<usize>::with_capacity(more_pages);
            for _i in 0..more_pages {
                frames.push(0); // Initialize to zero
            }
            let _ = PhysMem::alloc_high_frames(frames.as_mut_slice())?;
            // Map the additional frames from heap_base + current heap_pages
            let virt = proc.heap_base + (proc.heap_pages * PHY_FRAME_SIZE);
            let frms = frames.as_slice();
            proc.vmap.map_pages(virt, frms, true, false, MemoryType::Normal);
            // Update the process's heap information
            proc.heap_pages = new_heap_pages;
            return Ok((proc.heap_base, new_heap_size));
        } else {
            // Shrink the heap
            let shrink_pages = (-delta) as usize / PHY_FRAME_SIZE;
            if shrink_pages > proc.heap_pages {
                // Cannot shrink more than the current heap size
                return Err(error!(ErrorCode::InvalidArgument));
            }
            // Unmap and free the pages being removed from the heap
            for i in 0..shrink_pages {
                let page_vaddr = proc.heap_base + ((proc.heap_pages - 1 - i)
                                                            * PHY_FRAME_SIZE);
                if let Some(page_paddr) = proc.vmap.unmap_page(page_vaddr) {
                    PhysMem::free(page_paddr);
                }
            }
            // Update the process's heap information
            proc.heap_pages -= shrink_pages;
            return Ok((proc.heap_base, proc.heap_pages * PHY_FRAME_SIZE));
        }
    }


    /// Expands the user stack of the currently running task by allocating
    /// `page_count` more pages and mapping them in the process's address space.
    pub fn expand_stack(&mut self, page_count: usize) -> Result<(), Error> {
        // Find the current user-task structure
        let tid = Task::current_tid();
        let Some(utask) = self.children.iter_mut().find(|t| t.tid == tid) else {
            return Err(error!(ErrorCode::InvalidTID));
        };
        dbg!("expand_stack: current: {} pages, top: {:X}, base: {:X}\n",
            utask.ustack_pages, utask.stack_top(), utask.ustack_base);
        // Calculate the new stack size after expansion and check if feasible
        let new_stack_pages = utask.ustack_pages + page_count;
        if  new_stack_pages > utask.ustack_max_pages {
            // Exceeds maximum stack size
            return Err(error!(ErrorCode::OutOfMemory));
        }

        if self.stack_top - (new_stack_pages * MMUMapping::PAGE_SIZE) <= 
                self.heap_base + (self.heap_pages * PHY_FRAME_SIZE) {
            //There isn't enough space to expand the stack without colliding
            // with the heap
            return Err(error!(ErrorCode::OutOfMemory));
        }
        // Allocate additional pages for the stack
        let mut frames = Vec::<usize>::with_capacity(page_count);
        for _i in 0..page_count {
            frames.push(0); // Initialize to zero
        }
        PhysMem::alloc_high_frames(frames.as_mut_slice())?;
        // Map the new stack pages in the process's address space
        let virt = utask.stack_top() - (page_count * MMUMapping::PAGE_SIZE);
        let frms = frames.as_slice();
        self.vmap.map_pages(virt, frms, true, false, MemoryType::Normal);
        // Update the task's stack information
        utask.ustack_pages += page_count;
        dbg!("expand_stack: now: {} pages, top: {:X}, base: {:X}\n",
            utask.ustack_pages, utask.stack_top(), utask.ustack_base);
        Ok(())
    }

    /// Maps a range of physical frames to the dynamic mapping pool (dmap) of
    /// the current process, and returns the base virtual address of the new
    /// mapping if successful.
    pub fn dmap(phys_addrs: &[usize]) -> Result<usize, Error> {
        let mut proc_map = PROCESSES.lock();
        let Some(proc) = proc_map.get_mut(&Task::current_pid()) else {
            return Err(error!(ErrorCode::InvalidPID));
        };
        if let Some(virt_base) = proc.vmap.dmap_pages(phys_addrs) {
            return Ok(virt_base);
        }
        Err(error!(ErrorCode::OutOfMemory))
    }

    pub fn dunmap(virt_addr: usize, page_count: usize) -> Result<(), Error> {
        let mut proc_map = PROCESSES.lock();
        let Some(proc) = proc_map.get_mut(&Task::current_pid()) else {
            return Err(error!(ErrorCode::InvalidPID));
        };
        if proc.vmap.unmap_pages(virt_addr, page_count) != page_count {
            return Err(error!(ErrorCode::Other)); // BUG!
        }
        Ok(())
    }
    //
    // Fault Handling methods
    //

    /// Handles a page fault at the given faulting virtual address in the
    /// context of the currently running process.
    /// Returns true if the page fault was handled successfully
    pub fn handle_page_fault(fault_addr: usize, instr: bool, _write: bool) 
                                                                    -> bool {
        // Handle task's exit handler (return to kernel)
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
        // Handle NX fetches, stack growth, etc. that requires process/utask
        // information retrieval.
        let pid = Task::current_pid();
        let tid = Task::current_tid();
        let mut proc_map = PROCESSES.lock();
        let Some(proc) = proc_map.get_mut(&pid) else {
            return false;
        };
        let Some(utask) = proc.children.iter_mut().find(|t| t.tid == tid) else {
            return false;
        };

        // Block No-Execute Instruction Fetches:
        // Check if the faulting address in an instruction fetch outside of the
        // program image, and block it if so.
        let prg_end = MMUMapping::MIN_VIRTUAL_USER as usize + 
                                            (proc.img_pages * PHY_FRAME_SIZE);
        if instr && fault_addr >= prg_end {
            return false;
        }
        
        // Stack Growth Handling:
        // Check if the faulting address is within the range for stack growth
        let lowest_stack_addr = utask.ustack_base + 1 - 
                            (utask.ustack_max_pages * MMUMapping::PAGE_SIZE);
        if fault_addr < utask.ustack_base && fault_addr > lowest_stack_addr {
            // Faulting address is within the stack growth range
            // Expand by smallest multiple of DEFAULT_USTACK_EXPAND_PAGES that
            // can accomodate the faulting address
            let multiple = DEFAULT_USTACK_EXPAND_PAGES.load(Ordering::Relaxed);
            let extra_pages = div_round_up!(utask.stack_top() - fault_addr,
                                MMUMapping::PAGE_SIZE * multiple) * multiple;
            dbg!("Page fault @{:X} within stack growth range [{:X}..{:X}] for \
                    PID {}, TID {} - CurTop:{:X}, Adding {} pages\n",
                    fault_addr, lowest_stack_addr, utask.ustack_base, pid, tid,
                    utask.stack_top(), extra_pages);

            if let Err(e) = proc.expand_stack(extra_pages) {
                klog!("Failed to expand stack for PID {}: {:?}\n", pid, e);
                return false;
            }
            MMUMapping::flush_tlbs();
            return true;
        }
        false
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
    }
}

unsafe extern "C" {
    unsafe static _KERNEL_TEXT_START: usize;
    unsafe static _KERNEL_TEXT_END: usize;
}
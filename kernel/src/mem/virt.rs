
use core::sync::atomic::AtomicUsize;

use alloc::collections::btree_map::BTreeMap;

use crate::arch::MMUMapping;
use crate::mem::phys::*;
//
// Process Address Space (Generic code)
//
use crate::util::*;
use crate::{arch, sched::Task};
use core::sync::atomic::Ordering;

static PROCESSES: Spinlock<BTreeMap<usize, AddressSpace>> = 
        Spinlock::new(BTreeMap::new());

static NEXT_PID: AtomicUsize = AtomicUsize::new(1);

pub struct AddressSpace {
    pid:            usize,
    // Architecture-dependent address-space object
    vmap:            MMUMapping,
    // Start & end physical address where the program image is loaded
    img_paddr:      (usize, usize),
    // Program's entry pont (virtual)
    ep_vaddr:       usize,
    root_tid:       usize, // Main task whose termination frees the process
    // User Stack (phys)
    usr_stk_base:   usize,
    usr_stk_pages:  usize,
    // Virtual address of the stack pointer
    usr_stk_vptr:   usize,

}

impl AddressSpace {
    pub const fn new() -> Self {
        Self {
            pid: 0,
            vmap: MMUMapping::new(),
            img_paddr: (0, 0),
            ep_vaddr: 0,
            usr_stk_pages: 0,
            usr_stk_base: 0,
            usr_stk_vptr: 0,
            root_tid: 0
        }
    }

    // Program image is already loaded and marked in the physical memory.
    // Just initialize the Process object fields
    // This can be called from any thread context as it doesn't perform any
    // mapping, stack allocation.
    // Adds the process to the process pool and returns the PID of the new
    // process address space
    pub fn spawn(img_phy_base: usize, img_phy_end: usize,
                ep_virt_addr: usize) -> Option<usize> {
        let mut proc_map = PROCESSES.lock();
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        proc_map.insert(pid, Self::new());
        let proc = proc_map.get_mut(&pid);
        match proc {
            Some(p) => {
                p.ep_vaddr      = ep_virt_addr;
                p.img_paddr.0   = img_phy_base;
                p.img_paddr.1   = img_phy_end;
                p.pid           = pid;
            }
            None    => return None
        }
        Some(pid)
    }

    //
    // Loads a process into the memory from the currenly running process
    // and then calls spawn and returns the PID of the new process address space
    pub fn fork() -> usize{
        panic!("Not supported!");
    }

    //
    // Loads a program image in the memory from a given ELF image and calls
    // spawn
    pub fn spawn_from_elf() -> usize {
        panic!("Not supported!");
    }

    // Must be called from the context of the kernel thread that is going to
    // be converted to the main thread of this user process.
    // It also migrates the task to the new process
    pub fn launch(pid: usize) {
        if pid == 0 {
            return;
        }
        let ep_vaddr;
        let usr_stk_vptr;
        let adp_priv;
        {
            let mut proc_map = PROCESSES.lock();
            let proc = proc_map.get_mut(&pid);
            match proc {
            Some(p) => {
                // Initialize the Arch-dependent PAS object (maps the kernel)
                p.vmap.init();
                // Map the initial user-spaec addresses
                p.vmap.map_pages(
                    MMUMapping::MIN_VIRTUAL as usize,
                    p.img_paddr.0, 
                    (p.img_paddr.1 - p.img_paddr.0) / MMUMapping::PAGE_SIZE,
                    false, true, true, arch::MmuCachingPolicy::WriteBack
                );
                // Allocate and map a 4-page user-stack at the end of the VAS range
                p.usr_stk_base = palloc_continuous(4).expect("Out of memory");
                p.usr_stk_pages = 4;
                p.usr_stk_vptr = MMUMapping::MAX_VIRTUAL as usize + 1;
                p.vmap.map_pages(
                    p.usr_stk_vptr - MMUMapping::PAGE_SIZE * 4,
                    p.usr_stk_base,
                    4, false, true, false, arch::MmuCachingPolicy::WriteBack
                );
                // Set the main/root task ID
                if p.root_tid == 0 {
                    p.root_tid = Task::current();
                }
                ep_vaddr        = p.ep_vaddr;
                usr_stk_vptr    = p.usr_stk_vptr;
                adp_priv        = p.vmap.copy_priv_data();
            }
            None    => {
                return;
            }
            }
        } // ProcessPool unlocked
        Task::migrate_to_process(pid);
        MMUMapping::move_to_userspace(adp_priv, ep_vaddr, usr_stk_vptr);
        panic!("Must have been unreachable!");
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
                if p.root_tid == tid {
                    proc_map.remove(&pid);
                }
            }
            None    => {}
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

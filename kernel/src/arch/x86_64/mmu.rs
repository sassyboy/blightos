// 
// BlightOS Kernel
// 
// Support module for the X64 Memory Management Unit
//     
// TODO: Set appropriate RWX flags for the varous kernel/user mappings
//
use core::arch::asm;
use crate::arch::{MMUTrait, x86_msr_read, x86_msr_write};
use crate::mem::{MemoryType, phys::*};
use crate::util::*;

#[cfg(feature="debug_mmu")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[MMU] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}
#[cfg(not(feature="debug_mmu"))]
macro_rules! dbg{
    ($($arg:tt)*) => { };
}

/// Provides the architecture-dependent implementation of the MMU functionality
/// for x64 to manage the virtual memory mappings for the kernel and user-space
/// processes.
/// 
/// Address-Space Configuration
///
/// KERN: Virt 0   to 4GB ----> Phys 0 to 4GB as WriteBack cachable memory
///       Virt 4GB to 8GB ----> Kernel's dynamic mapping area (kmap) for drivers
///
/// USER: Virt 8GB to 256GB---> palloc'ed frames at 4KB granularity
///       Program Image (from 8GB virt)
///       Program Heap
///            |
///            V
///            ^
///            |
///        User Stack <-- User Task N's
///            .
///            .
///        User Stack <-- Main Tasks (MAX_VIRTUAL_STACK)
///           dmap    <-- The very last 4GB for User's dynamic mapping
///            ^      <-- MAX_VIRTUAL
///
/// 4-Level Mapping - Virtual Address bits
///     9 bits       9 bits       9 bits         9 bits         12 bits
/// [47 pml4e 39][38 pdpte 30][29   pde   21][20   pte   12][11   off   0]
///
pub struct MMUMapping {
    // Virt=Phys address of PML4 Table <-> CR3
    pml4_base   : usize,
    // Virt=Phys address of PML4[0] -> PDPT0 Table base (first 512GB)
    pdpt0_base  : usize,
    // Number of pages mapped via map_pages (for testing/logging purposes)
    mapped_pages_count : usize,
    // Number of pages allocated for paging structures (for logging purposes)
    tlb_page_count : usize,
}

/// Gaurds any changes to the kernel's dynamic mapping area
/// (4GB to 8GB virtual address range)
static KMAP_LOCK: Spinlock<()> = Spinlock::new(());

impl MMUMapping {
    // See boot.S for our GDT Entries
    const GDTE_USER_CODE: u16 = 0x18;
    const GDTE_USER_DATA: u16 = 0x20;
    // Segment Selector Values: See Section 3.4.2 - Segment Selectors
    const SEGSEL_USER_CODE: u16 = Self::GDTE_USER_CODE | 0x3; // CPL: Ring 3
    const SEGSEL_USER_DATA: u16 = Self::GDTE_USER_DATA | 0x3; // CPL: Ring 3
    // Paging Structure Entry Definitions
    const PGENT_PRESENT:        u64 = 0x1;
    const PGENT_WRITABLE:       u64 = 0x2;
    const PGENT_USERMODE:       u64 = 0x4;
    const PGENT_PWT:            u64 = 0x8;  // Page-level Write-throuhg
    const PGENT_PCD:            u64 = 0x10; // Page-level Cache Disable
    const PGENT_PS:             u64 = 0x80; // Set for large pages
    const PGENT_G:              u64 = 0x100; // Global
    const PGENT_XD:             u64 = 1 << 63; // No-Execute
    const PGENT_BASE_MASK:      u64 = 0xFFFFFFF000;

    const EFER_MSR_ADDR: u32 = 0xC0000080;
    const EFER_NXE_BIT: u64 = 1 << 11;

    /// Minimum virtual address that can be used for dynamic kernel mappings
    pub const MIN_KPOOL_VIRTUAL: u64 = 0x100000000;
    /// Maximum virtual address that can be used for dynamic kernel mappings.
    pub const MAX_KPOOL_VIRTUAL: u64 = 0x200000000;
    pub const KPOOL_PAGES: usize = ((Self::MAX_KPOOL_VIRTUAL -
                                    Self::MIN_KPOOL_VIRTUAL) as usize) /
                                    Self::PAGE_SIZE;

    /// Minimum virtual address that can be used for non-priviledged
    /// (user-space) mappings. The kernel address space will be mapped
    /// from 0x0 to MIN_VIRTUAL_USER
    pub const MIN_VIRTUAL_USER:  u64= 0x200000000;          // @ 8GB
    pub const MAX_USTACK_VIRTUAL:u64= 0x7F00000000 - 1;     // @ 508GB - 1
    pub const MIN_DPOOL_VIRTUAL: u64= 0x7F00000000;         // @ 508GB usmap
    /// Last virtual address that can be mapped (only using PML4[0])
    pub const MAX_VIRTUAL:      u64 = 0x8000000000 - 1;     // @ 512GB - 1
    pub const PAGE_SIZE:        usize = 0x1000; // Only 4KB pages in this range
    
    pub const fn new() -> Self {
        Self {
            pml4_base: 0,
            pdpt0_base: 0,
            mapped_pages_count: 0,
            tlb_page_count: 0,
        }
    }

    /// Prints the present kernel mappings in the following form for debugging:
    /// virt. address --> phys. address (flags)
    pub fn log_kmap() {
        unsafe extern "C" {
            // 512x4 PTs for 4 PD covering kpool
            unsafe static kpg_tbls: usize; 
        }
        let pg_tbls_base = unsafe { &kpg_tbls as *const usize as usize };
        let mut pte_addr: *mut u64  = pg_tbls_base as *mut u64;
        let mut vaddr: usize = Self::MIN_KPOOL_VIRTUAL as usize;
        for _v in 0..Self::KPOOL_PAGES {
            let pte = unsafe { pte_addr.read_volatile() };
            if pte & Self::PGENT_PRESENT != 0 {
                let paddr = pte & Self::PGENT_BASE_MASK;
                klog!("KMAP: {:X} --> {:X} (flags={:X})\n", vaddr, paddr, pte);
            }
            vaddr += Self::PAGE_SIZE;
            pte_addr = unsafe { pte_addr.add(1) }; // Move to the next PTE 
        }
    }
    
    fn entry_flags(kern: bool, w: bool, x: bool, cache: &MemoryType) -> u64 {
        let mut flags = Self::PGENT_PRESENT;
        if w {
            flags |= Self::PGENT_WRITABLE;
        }
        if kern {
            flags |= Self::PGENT_G;
        } else {
            flags |= Self::PGENT_USERMODE;
        }
        match cache {
            MemoryType::Normal => { /* default is WriteBack */ },
            MemoryType::Device => { 
                flags |= Self::PGENT_PCD; 
            },
            MemoryType::OutputDMA => {
                // TODO: Add PAT entry for WriteCombining and use that instead
                // of WriteBack with PCD
                flags |= Self::PGENT_PWT; 
            },
        }
        if !x {
            flags |= Self::PGENT_XD;
        }
        flags
    }

    fn write_table_entry(table_virt_base: usize, index: usize, value: u64) {
        unsafe {
            let destp : *mut u64 = table_virt_base as *mut u64;
            *(destp.wrapping_add(index)) = value;
        }
    }

    fn read_table_entry(table_virt_base: usize, index: usize) -> u64 {
        unsafe {
            let destp : *mut u64 = table_virt_base as *mut u64;
            destp.wrapping_add(index).read_volatile()
        }
    }
    
    //
    // Misc.
    //
    pub fn mapped_pages_count(&self) -> usize {
        self.mapped_pages_count
    }

    pub fn tlb_page_count(&self) -> usize {
        self.tlb_page_count
    }

    pub fn log_mapping(&self, vaddr: usize) {
        let pdpt0e_i    = (vaddr >> 30) & 0x1FF; // Index in PDPT[0]
        let pde_i       = (vaddr >> 21) & 0x1FF; // Index in PD
        let pte_i       = (vaddr >> 12) & 0x1FF; // Index in PT

        
        klog!("pml4[0] @ {:X} => {:X}\n", self.pml4_base, 
            Self::read_table_entry(self.pml4_base, 0));

        let pdpte = Self::read_table_entry(self.pdpt0_base, pdpt0e_i);
        klog!("  pdpt0 @ {:X} elem[{}]=> {:X}\n", self.pdpt0_base, pdpt0e_i,
            pdpte as usize);

        let pd_base = pdpte & Self::PGENT_BASE_MASK;
        let pde = Self::read_table_entry(pd_base as usize, pde_i);
        klog!("    pd @ {:X} elem[{}]=> {:X}\n", pd_base, pde_i, pde);
        
        let pt_base = pde & Self::PGENT_BASE_MASK;
        let pte = Self::read_table_entry(pt_base as usize, pte_i);
        klog!("      pt @ {:X} elem[{}]=> {:X}\n", pt_base, pte_i, pte);
        klog!("VADDR {:X} --> PADDR {:X}\n", vaddr, 
                pte & Self::PGENT_BASE_MASK);
    }
}

impl MMUTrait for MMUMapping {

    //
    // MMU Initialization Methods
    //

    /// PDPT[0..3] are already identity-mapped to the first 4GB of physical
    /// memory for the kernel's code and data.
    /// This function is called by rust_x864_entry_bsp() to set up the kernel's
    /// dynamic mapping area so that device drivers (and other parts of the
    /// kernel) can use that area to map/unmap physical memory for DMA
    /// operations, physical memory access, etc.
    /// The dynamic mapping area covers [4GB, 8GB) virtual address range, which
    /// will be divided to 4KB pages and left unmapped (not present) until 
    /// a client requests it.
    fn global_init() {
        let pdpt0_base   = unsafe { &kpdpt0_tbl as *const usize as usize };
        let pd_tlbs_base = unsafe { &kpd_tlbs as *const usize as usize };
        let pg_tbls_base = unsafe { &kpg_tbls as *const usize as usize };
        // Populate pdpt0[3..7]
        for pdpt0e_i in 4..8 {
            // Set up the Page Directory for this 1GB region
            let pd_base = pd_tlbs_base + (pdpt0e_i as usize - 4)
                                                            * Self::PAGE_SIZE;
            for pde_i in 0..512 {
                // Set up the Page Table for this 2MB region
                let pt_base = pg_tbls_base + ((pdpt0e_i as usize - 4) * 512
                                            + pde_i as usize) * Self::PAGE_SIZE;
                // Zero out the page table as there are no mappings yet
                unsafe { (pt_base as *mut u8).write_bytes(0, 0x1000); }
                // Set the PDE to point to the page table
                Self::write_table_entry(pd_base, pde_i as usize,
                                        (pt_base as u64) | 
                                        Self::PGENT_PRESENT |
                                        Self::PGENT_WRITABLE | Self::PGENT_G);
            }
            // Set the PDPT entry to point to the page directory
            Self::write_table_entry(pdpt0_base, pdpt0e_i as usize,
                                    (pd_base as u64) | 
                                    Self::PGENT_PRESENT |
                                    Self::PGENT_WRITABLE | Self::PGENT_G);
        }
    }

    fn per_cpu_init() {
        // Nothing to do for x64 as the kernel's address space is shared
        // among all CPUs, and the paging structures are already set up in
        // global_init. Just need to enable NXE bit in EFER MSR to support
        // No-Execute flag in page table entries.
        let efer = x86_msr_read(Self::EFER_MSR_ADDR);
        x86_msr_write(Self::EFER_MSR_ADDR, efer | Self::EFER_NXE_BIT);
    }

    //
    // Kernel Dynamic Mapping Methods
    //

    /// Given a physical address and a frame count, finds a virtual address
    /// range in the kernel's dynamic mapping area and maps the physical frames
    /// to that virtual address range with the appropriate flags for the given
    /// cache type, and returns the base virtual address of the mapped range.
    /// The resulting mapping is continuous both in virtual and physical memory.
    /// The caller is expected to call kunmap_frames to unmap the virtual
    /// address range and free the physical frames back to the physical memory
    /// manager.
    /// 
    /// Note that the mapping is not tied to any address-space and is shared
    /// among all address spaces since it belongs to the kernel.
    fn kmap(phys_base: usize, frame_cnt: usize, cache: MemoryType)
                                                            -> Option<usize> {
        let pg_tbls_base = unsafe { &kpg_tbls as *const usize as usize };
        // Can't use map_pages since that methods works for user-space processes
        let mut vaddr: usize        = Self::MIN_KPOOL_VIRTUAL as usize;
        let mut pte_addr: *mut u64  = pg_tbls_base as *mut u64;
        KMAP_LOCK.lock();
        for _v in 0..Self::KPOOL_PAGES {
            let mut found = true;
            for w in 0..frame_cnt {
                let pte = unsafe { pte_addr.add(w).read_volatile() };
                if pte & Self::PGENT_PRESENT != 0 {
                    // This page is already mapped, try the next one
                    found = false;
                    break;
                }
            }
            if found {
                // Found a contiguous range of free pages
                for w in 0..frame_cnt {
                    let pte_flags = Self::entry_flags(true, true, false, &cache);
                    let paddr = (phys_base + w * Self::PAGE_SIZE) as u64;
                    unsafe {
                        pte_addr.add(w).write_volatile(paddr | pte_flags);
                    }
                    Self::flush_tlb_for_page(vaddr + w * Self::PAGE_SIZE);
                }
                return Some(vaddr);
            }
            vaddr += Self::PAGE_SIZE;
            pte_addr = unsafe { pte_addr.add(1) }; // Move to the next PTE 
        }
        None
    }

    fn kunmap(virt_base: usize, frame_cnt: usize) {
        if virt_base < Self::MIN_KPOOL_VIRTUAL as usize ||
                virt_base >= Self::MAX_KPOOL_VIRTUAL as usize {
            return;
        }
        let pg_tbls_base = unsafe { &kpg_tbls as *const usize as usize };
        let pte_addr: *mut u64  = pg_tbls_base as *mut u64;
        let start_page_index = (virt_base - Self::MIN_KPOOL_VIRTUAL as usize)
                                / Self::PAGE_SIZE;
        KMAP_LOCK.lock();
        for pe_i in start_page_index..start_page_index + frame_cnt {
            unsafe {
                pte_addr.add(pe_i).write_volatile(0);
            }
        }
        Self::flush_tlbs();
    }

    //
    // User-Space Mapping Methods
    //

    /// Creates the initial paging structures for the process.
    /// The first 8GBs are formed from kernel's mappings.
    /// [0GB to 4GB] is mapped as 1GB pages for the kernel's code/data
    /// [4GB to 8GB] point to the same page directories kernel maintains for its
    ///              dynamic mapping pool. Any mapping done by the kernel in
    ///              that pool will be visible in the
    fn init(&mut self) {
        // Allocate and zero out:
        //   1 page for the PML4 table and 1 page for the first PDPT table
        self.pml4_base  = PhysMem::alloc().expect("Out of memory");
        self.pdpt0_base = PhysMem::alloc().expect("Out of memory");
        unsafe {
            (self.pml4_base  as *mut u8).write_bytes(0, 0x1000);
            (self.pdpt0_base as *mut u8).write_bytes(0, 0x1000);
        }

        // Set PML4[0] --> PDPT0 that covers the first 512 GB
        let pml4e0 = self.pdpt0_base as u64 | Self::PGENT_PRESENT |
                    Self::PGENT_WRITABLE | Self::PGENT_USERMODE;
        Self::write_table_entry(self.pml4_base, 0, pml4e0);

        // Kernel's code/data 
        // Set PDPT0[0..=3] --> PHYS[0GB to 4GB] as writable+cachable
        let pdpt0e = Self::PGENT_PRESENT | Self::PGENT_WRITABLE |
                        Self::PGENT_PS | Self::PGENT_G; // 1GB page
        for i in 0..4 {
            let phys_addr : u64 = i << 30;
            Self::write_table_entry(self.pdpt0_base, i as usize, 
                                    pdpt0e | phys_addr);
        }
        // Kernel's dynamic mapping area (kmap)
        // Set PDPT0[4..=7] --> kpd_tlbs[0..3]
        let pd_tlbs_base = unsafe { &kpd_tlbs as *const usize as usize };
        for i in 4..8 {
            let pd_base = pd_tlbs_base + (i as usize - 4) * Self::PAGE_SIZE;
            Self::write_table_entry(self.pdpt0_base, i as usize,
                                    (pd_base as u64) | 
                                    Self::PGENT_PRESENT |
                                    Self::PGENT_WRITABLE |
                                    Self::PGENT_G);
        }

        // Log everything for testing
        // klog!("PML4 Base: {:X}, PML4E0: {:X}\n",
        //     self.pml4_base,
        //     Self::read_table_entry(self.pml4_base, 0)
        // );
        // klog!("PDPT0 Base: {:X}\n", self.pdpt0_base);
        // for _i in 0..8 {
        //     klog!("    PDPT0[{}] : {:X}\n", _i,
        //             Self::read_table_entry(self.pdpt0_base, _i));
        // }
        self.tlb_page_count = 2; // PML4 + PDPT0
    }

    /// Maps a number of pages (virtual address) to frames (physical address)
    /// for a user-space process.
    /// The assumption is that the caller has already reserved page_cnt frames
    /// starting from phys_address from the physical memory manager.
    /// If the virtual address is already mapped, this function will return
    /// false without modifying the existing mapping.
    fn map_pages(&mut self, virt_addr: usize, phys_addrs: &[usize],
                     writeable: bool, exec: bool, cache: MemoryType) -> bool {
        // Page-align the given starting virtual address
        let mut vaddr : u64 = virt_addr as u64 & Self::PGENT_BASE_MASK;
        if (vaddr < Self::MIN_VIRTUAL_USER) || vaddr > Self::MAX_VIRTUAL  {
            return false;
        }

        let flgs_top = Self::entry_flags(false, true, true, &MemoryType::Normal);
        let flgs_pge = Self::entry_flags(false, writeable, exec, &cache);

        for i in 0..phys_addrs.len() {
            let pdpt0e_i    = (vaddr >> 30) & 0x1FF; // Index in PDPT[0]
            let pde_i       = (vaddr >> 21) & 0x1FF; // Index in PD
            let pte_i       = (vaddr >> 12) & 0x1FF; // Index in PT
            let pd_base : usize;
            let pt_base : usize;
            let paddr : u64 = phys_addrs[i] as u64 & Self::PGENT_BASE_MASK;

            // 1GB Region
            let mut pdpt0e = Self::read_table_entry(self.pdpt0_base,
                                                pdpt0e_i as usize);
            if pdpt0e == 0 {
                // Allocate a page directory for this PDPT0[pdpt0e_i]
                pd_base = PhysMem::alloc().expect("Out of memory");
                self.tlb_page_count += 1;
                unsafe {
                    (pd_base as *mut u8).write_bytes(0, 0x1000);
                }
                pdpt0e = pd_base as u64 | flgs_top;
                Self::write_table_entry(self.pdpt0_base,
                                        pdpt0e_i as usize, pdpt0e);
            } else {
                // Retrieve the page directory's base from PDPT0[pdpt0e_i]
                pd_base = (pdpt0e & Self::PGENT_BASE_MASK) as usize;
            }

            // 2 MB Region
            let mut pde = Self::read_table_entry(pd_base,
                                            pde_i as usize);
            if pde == 0 {
                // Allocate a page table for this PD[pde_i]
                pt_base = PhysMem::alloc().expect("Out of memory");
                self.tlb_page_count += 1;
                unsafe {
                    (pt_base as *mut u8).write_bytes(0, 0x1000);
                }
                pde = pt_base as u64 | flgs_top;
                Self::write_table_entry(pd_base, pde_i as usize, pde);
            } else {
                // Retrieve the page table's base from ths PD[pde_i]
                pt_base = (pde & Self::PGENT_BASE_MASK) as usize;
            }

            // 4KB Region
            let mut pte = Self::read_table_entry(pt_base, pte_i as usize);
            if pte & Self::PGENT_PRESENT != 0 {
                panic!("map_pages - Virtual address {:X} is already mapped!\n",
                    virt_addr);
            }
            pte = (paddr & Self::PGENT_BASE_MASK) | flgs_pge;
            Self::write_table_entry(pt_base, pte_i as usize, pte);

            Self::flush_tlb_for_page(vaddr as usize);
            vaddr += Self::PAGE_SIZE as u64;
        }
        self.mapped_pages_count += phys_addrs.len();
        true
    }

    fn dmap_pages(&mut self, phys_addrs: &[usize]) -> Option<usize> {
        let num_pages = phys_addrs.len();
        // Find num_pages that aren't mapped starting from MIN_DPOOL_VIRTUAL
        // to MAX_VIRTUAL
        let mut vaddr_start = Self::MIN_DPOOL_VIRTUAL as usize;
        while vaddr_start < Self::MAX_VIRTUAL as usize {
            let mut found = true;
            for w in 0..num_pages {
                let vaddr = vaddr_start as usize + w * Self::PAGE_SIZE;
                if !self.virt_to_phys_from_map(vaddr).is_none() {
                    // Alread mapped => skip it
                    found = false;
                    break;
                }
            }
            if found {
                if !self.map_pages(vaddr_start, phys_addrs, true, false, 
                                                        MemoryType::Normal){
                    klog!("BUG - map_pages failed in usmap_pages.");
                    return None;
                }
                return Some(vaddr_start);
            }
            vaddr_start += Self::PAGE_SIZE;
        }
        //
        None
    }

    ///
    /// Unmaps the given virtual address and returns the physical address of the
    /// page that was unmapped. The caller is expected to free the physical page
    /// after unmapping.
    /// 
    fn unmap_page(&mut self, virt_addr: usize) -> Option<usize> {
        let pdpt0e_i    = (virt_addr >> 30) & 0x1FF; // Index in PDPT[0]
        let pde_i       = (virt_addr >> 21) & 0x1FF; // Index in PD
        let pte_i       = (virt_addr >> 12) & 0x1FF; // Index in PT

        let pdpte = Self::read_table_entry(self.pdpt0_base, pdpt0e_i);
        if pdpte & Self::PGENT_PRESENT == 0 {
            return None;
        }
        let pd_base = pdpte & Self::PGENT_BASE_MASK;
        let pde = Self::read_table_entry(pd_base as usize, pde_i);
        if pde & Self::PGENT_PRESENT == 0 {
            return None;
        }
        let pt_base = pde & Self::PGENT_BASE_MASK;
        let pte = Self::read_table_entry(pt_base as usize, pte_i);
        if pte & Self::PGENT_PRESENT == 0 {
            return None;
        }

        // Unmap the page by clearing the Present bit in the PTE and return the
        // physical address of the page that was unmapped.
        Self::write_table_entry(pt_base as usize, pte_i, 
                                                    pte & !Self::PGENT_PRESENT);
        Self::flush_tlb_for_page(virt_addr);
        self.mapped_pages_count -= 1;
        Some((pte & Self::PGENT_BASE_MASK) as usize)
    }

    /// Unmaps the given virtual address range and returns the number of pages
    /// that were unmapped. The caller is expected to free the physical pages
    /// after unmapping.
    fn unmap_pages(&mut self, virt_addr: usize, num_pages: usize) -> usize {
        let mut unmapped = 0;
        for i in 0..num_pages {
            if self.unmap_page(virt_addr + (i * Self::PAGE_SIZE)) != None {
                unmapped += 1;
            }
        }
        unmapped
    }

    //
    // Address-space Switching Methods
    //

    /// Switches to the address space represented by this MMUMapping
    fn enter(&self) {
        unsafe {
            asm!("mov cr3, {}", in(reg) self.pml4_base);
        }
    }
    
    ///
    /// Converts the currently running kernel task into a user-space task as a
    /// part of this process address space. The calling (kernel) task will not
    /// return to the next instruction after its call to move_to_userspace.
    /// The user-space execution must end with an Exit system call, at which
    /// point the task terminates.
    /// 
    fn move_to_userspace(priv_data: usize, entry_point: usize, arg: usize,
                            user_stack: usize, exit_handler: fn()) -> ! {
        // Prepare CS, DS, SS for ring 3 transition and then jump to the
        // entry point address given. x64 doesn't support ljmp, so Iretq it is!
        // The context switch logic takes care of RSP0
        dbg!("Moving to user-space w\\ CR3={:X},Tcpu={}, RSP0={:X}, RSP3={:X}\n",
            priv_data, crate::arch::cpu_id(),
            crate::arch::cpu_stack_pointer(), user_stack);
        // Leave space for the return RIP for the exit handler
        let rsp3 = user_stack - 8;
        unsafe {
            // Move the the new address space by loading the new PML4 base into
            // CR3 and then push the address of our exit_handler as the return
            // RIP for the user-space code. The user-space task tries to return
            // to our exit_handler, but it page-faults, which is then caught
            // by the kernel (AddressSpace::handle_page_fault), which in turn
            // calls the exit_handler to clean up and exit the task.
            asm!(
                "cli", // Clear interrupts before switching to userspace
                "mov    cr3, {pml4_base}",
                "mov    [{user_rsp}], {exit_handler}",
                // IRERQ Frame
                "push   {stack_seg}",   // Ring-3 Stack Segment
                "push   {user_rsp}",    // Ring-3 Stack Pointer
                "push   {rflags}",      // Ring-3 Starting RFLAGS"
                "push   {code_seg}",    // Ring-3 Code Segment
                "push   {entry_point}", // Ring-3 RIP (Entry Point)
                "mov    rdi, {entry_point_arg}",
                "iretq",
                pml4_base       = in(reg) priv_data,
                user_rsp        = in(reg) rsp3,
                exit_handler    = in(reg) exit_handler as usize,
                stack_seg       = const (0x20 | 3) as usize,
                rflags          = const 0x202 as usize,
                code_seg        = const (0x18 | 3) as usize,
                entry_point     = in(reg) entry_point,
                entry_point_arg = in(reg) arg
            );
        }
        panic!("Must have been unreachable!\n");
    }

    fn copy_priv_data(&self) -> usize {
        self.pml4_base
    }

    //
    // Misc Methods
    //

    /// Invalidate the TLB entries for the entire address space by reloading
    /// CR3 with the same value. This is a simple but expensive way to flush
    /// the TLB.
    fn flush_tlbs() {
        unsafe {
            let cr3: usize;
            asm!("mov {}, cr3", out(reg) cr3);
            asm!("mov cr3, {}", in(reg) cr3);
        }
    }

    /// Invalidate the TLB entry for the given virtual address using the INVLPG
    /// instruction. This is more efficient than flushing the entire TLB when
    /// only a few pages are unmapped/changed.
    fn flush_tlb_for_page(virt_addr: usize) {
        unsafe {
            asm!("invlpg [{}]", in(reg) virt_addr,
            options(nostack, preserves_flags));
        }
    }

    /// virtual->physical address translation for self
    fn virt_to_phys_from_map(&self, virt_addr: usize) -> Option<usize> {
        let pdpt0e_i    = (virt_addr >> 30) & 0x1FF; // Index in PDPT[0]
        let pde_i       = (virt_addr >> 21) & 0x1FF; // Index in PD
        let pte_i       = (virt_addr >> 12) & 0x1FF; // Index in PT

        let pdpte = Self::read_table_entry(self.pdpt0_base, pdpt0e_i);
        if pdpte & Self::PGENT_PRESENT == 0 {
            return None;
        }
        let pd_base = pdpte & Self::PGENT_BASE_MASK;
        let pde = Self::read_table_entry(pd_base as usize, pde_i);
        if pde & Self::PGENT_PRESENT == 0 {
            return None;
        }
        let pt_base = pde & Self::PGENT_BASE_MASK;
        let pte = Self::read_table_entry(pt_base as usize, pte_i);
        if pte & Self::PGENT_PRESENT == 0 {
            return None;
        }

        Some((pte & Self::PGENT_BASE_MASK) as usize)
    }

    /// virtual->physical address translation for the caller's address-space
    fn virt_to_phys(virt_addr: usize) -> Option<usize> {
        let pml4_base: usize;
        unsafe {
            asm!("mov {}, cr3", out(reg)pml4_base);
        }
        
        let pdpt0_base  = Self::read_table_entry(pml4_base, 0) 
                                                    & Self::PGENT_BASE_MASK;
        let pdpt0e_i    = (virt_addr >> 30) & 0x1FF; // Index in PDPT[0]
        let pde_i       = (virt_addr >> 21) & 0x1FF; // Index in PD
        let pte_i       = (virt_addr >> 12) & 0x1FF; // Index in PT
        let pdpte = Self::read_table_entry(pdpt0_base as usize, pdpt0e_i);
        if pdpte & Self::PGENT_PRESENT == 0 {
            return None;
        }
        let pd_base = pdpte & Self::PGENT_BASE_MASK;
        let pde = Self::read_table_entry(pd_base as usize, pde_i);
        if pde & Self::PGENT_PRESENT == 0 {
            return None;
        }
        let pt_base = pde & Self::PGENT_BASE_MASK;
        let pte = Self::read_table_entry(pt_base as usize, pte_i);
        if pte & Self::PGENT_PRESENT == 0 {
            return None;
        }

        Some((pte & Self::PGENT_BASE_MASK) as usize)
    }

}

impl Drop for MMUMapping {
    fn drop(&mut self) {
        let mut _pg_count = 0;
        let mut _pg_st_count = 0;
        // Release the paging structures
        for pdpt0e_i in 0..512 {
            let pdpt0e = Self::read_table_entry(self.pdpt0_base, pdpt0e_i);
            if pdpt0e & Self::PGENT_PRESENT > 0 && pdpt0e & Self::PGENT_PS == 0{
                // Points to a page directory
                let pde_base = pdpt0e & Self::PGENT_BASE_MASK;
                for pde_i in 0..512 {
                    let pde = Self::read_table_entry(pde_base as usize, pde_i);
                    if pde & Self::PGENT_PRESENT > 0 &&
                        pde & Self::PGENT_PS == 0 {
                        // Points to a page table
                        let pt_base = (pde & Self::PGENT_BASE_MASK) as usize;
                        for pte_i in 0..512 {
                            let pte = Self::read_table_entry(pt_base, pte_i);
                            if pte & Self::PGENT_PRESENT > 0 {
                                let vaddr = pdpt0e_i << 30 |
                                            pde_i << 21 | pte_i << 12;
                                // Only free the physical frames private to the
                                // user address space!
                                // Don't free the kernel frames or dmap frames
                                if vaddr > Self::MIN_VIRTUAL_USER as usize && 
                                    vaddr < Self::MIN_DPOOL_VIRTUAL as usize {
                                    // Release the physical frame itself too
                                    _pg_count += 1;
                                    let addr = pte & Self::PGENT_BASE_MASK;
                                    PhysMem::free(addr as usize);
                                }
                                
                            }
                        }
                        // Release the Page Table
                        _pg_st_count += 1;
                        PhysMem::free((pde & Self::PGENT_BASE_MASK) as usize);
                    }
                }
                // Release the Page Directory
                _pg_st_count += 1;
                PhysMem::free(pde_base as usize);
            }
        }
        _pg_st_count += 2;
        PhysMem::free(self.pdpt0_base);
        PhysMem::free(self.pml4_base);
        dbg!("Released {} user frames and {} paging structure frames - \
              Free frames: {}\n", _pg_count, _pg_st_count,
              PhysMem::free_frame_count());

    }
}

unsafe extern "C" {
    // Kernel's PDPT0 table for the first 512GB virtual address space
    unsafe static kpdpt0_tbl: usize;
    // 4   PDs for the dynamic pool
    unsafe static kpd_tlbs: usize;
    // 512 PTs for each PD above linearly covering the entire dynamic pool
    unsafe static kpg_tbls: usize; 
}


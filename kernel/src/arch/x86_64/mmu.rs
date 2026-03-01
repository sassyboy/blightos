// 
// BlightOS Kernel
// 
// Support module for the X64 Memory Management Unit
//     
// TODO: unmapping, tlb invalidation

use core::arch::asm;
use crate::arch::MmuCachingPolicy;
use crate::mem::phys::*;
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

//
// Address-Space Configuration
//
// KERN: Virt 0   to 4GB ----> Phys 0 to 4GB as WriteBack cachable memory
//       Virt 4GB to 8GB ----> Phys 0 to 4GB as Non-cachable memory for DMA
//
// USER: Virt 8GB to 256GB---> palloc'ed frames at 4KB granularity
//
// 4-Level Mapping - Virtual Address bits
//     9 bits       9 bits       9 bits         9 bits         12 bits
// [47 pml4e 39][38 pdpte 30][29   pde   21][20   pte   12][11   off   0]
//


pub struct MMUMapping {
    // Virt=Phys address of PML4 Table <-> CR3
    pml4_base   : usize,
    // Virt=Phys address of PML4[0] -> PDPT0 Table base (first 512GB)
    pdpt0_base  : usize,
    // Number of pages mapped via map_pages (for testing/logging purposes)
    mapped_pages_count : usize,
    // Number of pages allocated for paging structures (for testing/logging purposes)
    tlb_page_count : usize,
}
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
    const PGENT_BASE_MASK:      u64 = 0xFFFFFFF000;

    
    // Virtual memory address range that can be mapped via calls to map_pages
    pub const MIN_VIRTUAL:      u64 = 0x200000000;     // 8 GBs
    pub const MAX_VIRTUAL:      u64 = (1 << 39) - 1;   // 256 GBs - only pml4[0]
    pub const PAGE_SIZE:        usize = 0x1000; // Only 4KB pages in this range
    
    pub const fn new() -> Self {
        Self {
            pml4_base: 0,
            pdpt0_base: 0,
            mapped_pages_count: 0,
            tlb_page_count: 0,
        }
    }

    // Creates the initial paging structures for the process that includes
    // the kernel mappings.
    // VIRTUAL MEM           --> PHYSICAL MEM
    // 0   - 4GB             --> 0 - 4GB R/W KERNEL MODE
    // 4GB - 8GB             --> 0 - 4GB but as DMA memory
    // 8GB - 8GB+IMAGE_SIZE  --> IMAGE_START + IMAGE_END
    pub fn init(&mut self) {
        // Allocate and zero out:
        //   1 page for the PML4 table and 1 page for the first PDPT table
        self.pml4_base  = palloc().expect("Out of memory");
        self.pdpt0_base = palloc().expect("Out of memory");
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
        // Kernel's DMA access
        // Set PDPT0[4..=7] --> PHYS[0GB to 4GB] as writable+non-cachable
        let pdpt0e = Self::PGENT_PRESENT | Self::PGENT_WRITABLE |
                        Self::PGENT_PCD | Self::PGENT_PS | Self::PGENT_G; // 1GB
        for i in 4..8 {
            let phys_addr : u64 = (i - 4) << 30;
            Self::write_table_entry(self.pdpt0_base, i as usize, 
                                    pdpt0e | phys_addr);
        }

        // Log everything for testing
        dbg!("PML4 Base: {:X}, PML4E0: {:X}\n",
            self.pml4_base,
            Self::read_table_entry(self.pml4_base, 0)
        );
        dbg!("PDPT0 Base: {:X}\n", self.pdpt0_base);
        for _i in 0..8 {
            dbg!("    PDPT0[{}] : {:X}\n", _i,
                    Self::read_table_entry(self.pdpt0_base, _i));
        }
        self.tlb_page_count = 2; // PML4 + PDPT0
    }

    //
    // Paging structure management methods 
    //
    // Virtual Address ----> Physical Address translation
    // 4GB - 0         ----> 4GB - 0 as four 1GB pages as supervisor access
    // Above 4GB       ----> Non-contiguous 4KB physical pages as user access
    //
    //

    pub fn addr_to_page_index(addr: usize) -> usize {
        addr >> 12
    }

    // Maps a page (virtual address > 4GB) to a frame (physical address)
    // The assumption is that the caller has already reserved page_cnt frames
    // starting from phys_address from the physical memory manager.
    pub fn map_pages(&mut self, virt_addr: usize, phys_addr: usize, page_cnt: usize,
                     privileged: bool, writeable: bool, _executable: bool,
                    _caching: MmuCachingPolicy) -> bool {
        // Page-align the given addresses
        let mut vaddr : u64 = virt_addr as u64 & Self::PGENT_BASE_MASK;
        let mut paddr : u64 = phys_addr as u64 & Self::PGENT_BASE_MASK;
        dbg!("map_pages(v:{:X}, p:{:X}, cnt:{}\n", vaddr, paddr, page_cnt);
        if vaddr < Self::MIN_VIRTUAL || vaddr > Self::MAX_VIRTUAL  {
            return false;
        }

        for _i in 0..page_cnt {
            let pdpt0e_i    = (vaddr >> 30) & 0x1FF; // Index in PDPT[0]
            let pde_i       = (vaddr >> 21) & 0x1FF; // Index in PD
            let pte_i       = (vaddr >> 12) & 0x1FF; // Index in PT
            let pd_base : usize;
            let pt_base : usize;

            // 1GB Region
            let mut pdpt0e = Self::read_table_entry(self.pdpt0_base,
                                                pdpt0e_i as usize);
            if pdpt0e == 0 {
                // Allocate a page directory for this PDPT0[pdpt0e_i]
                pd_base = palloc().expect("Out of memory");
                self.tlb_page_count += 1;
                unsafe {
                    (pd_base as *mut u8).write_bytes(0, 0x1000);
                }
                pdpt0e = pd_base as u64 | Self::PGENT_PRESENT |
                           Self::PGENT_USERMODE | Self::PGENT_WRITABLE;
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
                pt_base = palloc().expect("Out of memory");
                self.tlb_page_count += 1;
                unsafe {
                    (pt_base as *mut u8).write_bytes(0, 0x1000);
                }
                pde = pt_base as u64 | Self::PGENT_PRESENT |
                           Self::PGENT_USERMODE | Self::PGENT_WRITABLE;
                Self::write_table_entry(pd_base, pde_i as usize, pde);
            } else {
                // Retrieve the page table's base from ths PD[pde_i]
                pt_base = (pde & Self::PGENT_BASE_MASK) as usize;
            }

            // 4KB Region
            let mut pte = Self::read_table_entry(pt_base, pte_i as usize);
            if pte & Self::PGENT_PRESENT != 0 {
                klog!("map_pages - Virtual address {:X} is already mapped!\n",
                    virt_addr);
                return false;
            }
            pte = (paddr & Self::PGENT_BASE_MASK) | Self::PGENT_PRESENT;
            if writeable == true {
                pte |= Self::PGENT_WRITABLE;
            }
            if privileged == false {
                pte |= Self::PGENT_USERMODE;
            }
            Self::write_table_entry(pt_base, pte_i as usize, pte);

            vaddr += Self::PAGE_SIZE as u64;
            paddr += Self::PAGE_SIZE as u64;
        }
        self.mapped_pages_count += page_cnt;
        true
    }

    ///
    /// Unmaps the given virtual address and returns the physical address of the
    /// page that was unmapped. The caller is expected to free the physical page
    /// after unmapping.
    /// 
    pub fn unmap_page(&mut self, virt_addr: usize) -> Option<usize> {
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
        Self::write_table_entry(pt_base as usize, pte_i, pte & !Self::PGENT_PRESENT);
        self.mapped_pages_count -= 1;
        Some((pte & Self::PGENT_BASE_MASK) as usize)
    }

    /// Unmaps the given virtual address range and returns the number of pages
    /// that were unmapped. The caller is expected to free the physical pages
    /// after unmapping.
    pub fn unmap_pages(&mut self, virt_addr: usize, num_pages: usize) -> usize {
        let mut unmapped = 0;
        for i in 0..num_pages {
            if self.unmap_page(virt_addr + (i * Self::PAGE_SIZE)) != None {
                unmapped += 1;
            }
        }
        unmapped
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

    /*
     * Execution/Segmentation Management methods
     */
    ///
    /// Converts the currently running kernel task into a user-space task as a
    /// part of this process address space. The calling (kernel) task will not
    /// return to the next instruction after its call to move_to_userspace.
    /// The user-space execution must end with an Exit system call, at which
    /// point the task terminates.
    /// 
    pub fn move_to_userspace(priv_data: usize, entry_point: usize, arg: usize,
                            user_stack: usize, exit_handler: fn()) {
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

    pub fn copy_priv_data(&self) -> usize {
        self.pml4_base
    }

    /*
     * DMA
     */
    pub fn dma_from_kernel_phys(phys_addr: usize) -> usize{
        phys_addr | ((1 as usize) << 32)
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
                                // Release the physical frame itself too
                                _pg_count += 1;
                                pfree((pte & Self::PGENT_BASE_MASK) as usize);
                            }
                        }
                        // Release the Page Table
                        _pg_st_count += 1;
                        pfree((pde & Self::PGENT_BASE_MASK) as usize);
                    }
                }
                // Release the Page Directory
                _pg_st_count += 1;
                pfree(pde_base as usize);
            }
        }
        _pg_st_count += 2;
        pfree(self.pdpt0_base);
        pfree(self.pml4_base);
        dbg!("Released {} user frames and {} paging structure frames - \
              Free frames: {}\n", _pg_count, _pg_st_count,
              pmm_num_free_frames());

    }
}

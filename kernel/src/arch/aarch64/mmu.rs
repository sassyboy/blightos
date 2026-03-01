// 
// BlightOS Kernel
// 
// Support module for the AARCH64 Memory Management Unit
//      VMSAv8-64 - Non-secure EL1&0 translation regime
//
// Ref: The AArch64 Virtual Memory System Architecture
//      Chapter D8 of ARM Architecture Reference Manual (A-Profile)
//
// See "D8.2.8.2 VMSAv8-64 Stage 2 address translation using the 4KB 
// translation granule" for a quick galance of paging structure
//
// TODO: See Use of ASIDs and VMIDs to reduce TLB maintenance requirements.
//

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
// KERN: 0000_0000_0000_0000 to 0000_000F_FFFF_FFFF virt. address-space 
//       pointed to by TTBR0_EL1 (3 level paging, 64GB)
//                     ^^^^^^^^ <- same for all CPUs and address-spaces
//
// USER: FFFF_FFF0_0000_0000 to FFFF_FFFF_FFFF_FFFF virt. address-space
//       pointed to by TTBR1_EL1 (3-level paging, 64GB)
//                     ^^^^^^^^^ <- allocate/initialized for every process
//
// 3-Level Mapping - Virtual Address bits
//     9 bits       9 bits       9 bits         12 bits
// [38 lvl1 30][29   lvl2   21][20   lvl3   12][11   off   0]
//

pub struct MMUMapping {
    // Translation Table Base Register for User Space
    ttbr1:  u64,
    // Number of pages mapped via map_pages (for testing/logging purposes)
    mapped_pages_count : usize,
    // Number of pages allocated for paging structures (for testing/logging purposes)
    tlb_page_count : usize,
}
impl MMUMapping {
    
    // Index of different memory types/attributes we define for the MMU
    // MT_NORMAL: Inner/outer write-back cacheable memory with read/write hint
    //            and without transient hint
    // nG: No gathering (merging) of memory transactions
    // nR: No reordering of memory transactions
    // nE: No early acknowledgement of write requests (similar to WriteThrough)
    const MT_NORMAL:            usize = 0; // Regular memory
    const MT_NORMAL_NO_CACHING: usize = 1; // Not sure if need this
    const MT_DEVICE_NGNRNE:     usize = 2; // R/W DMA/Device
    const MT_DEVICE_NGNRE:      usize = 3; // Writeback DMA/Device

    // Virtual memory address range that can be mapped via calls to map_pages
    pub const MIN_VIRTUAL:      u64 = 0xFFFF_FFF0_0000_0000;
    pub const MAX_VIRTUAL:      u64 = 0xFFFF_FFFF_FFFF_0000 - 1;
    pub const PAGE_SIZE:        usize = 0x1000;         // Only 4KB pages
    const PAGE_BASE_MASK:       u64 = 0xFFFF_FFFF_FFFF_F000;
    // Paging Structure Entry Definitions
    const PGENT_BLK_DESC:       u64 = 0x1;
    const PGENT_TBL_DESC:       u64 = 0x3;
    const PGENT_PG_DESC:        u64 = 0x3;
    const PGENT_USERMODE:       u64 = 0x40;
    const PGENT_RO:             u64 = 0x80; // Read-only
    const PGENT_INSH:           u64 = 0x300; // Inner Sharable
    const PGENT_ACCESS:         u64 = 0x400; // Access flag must be set
    const PGENT_PHYS_ADDR_MASK: u64 = 0xFFFF_FFFF_F000;
    
    pub const fn new() -> Self {
        Self {
            ttbr1:       0,
            mapped_pages_count: 0,
            tlb_page_count: 0,
        }
    }

    // Called once for the whole system before application processors start
    pub fn global_init() {
        // Make sure the Maximum Physical Address >= 64GB
        let mem_model: u64;
        unsafe {
            asm!("mrs {0}, ID_AA64MMFR0_EL1", out(reg)mem_model);
        }
        if mem_model & 0xF < 1 { /* ID_AA64MMFR0_EL1.PARange */
            panic!("36-bit (64GB) Physical address not supported\n");
        }
    }

    // Called once for each CPU by the architecture stub code to perform 1-time 
    // checks and configurations on the current CPU
    pub fn percpu_init() {
        // Invalidate local TLB
        unsafe {
            asm!(
                "tlbi	vmalle1",
                "dsb	nsh"
            );
        }
	
        // Set up the memory types we use (see MT_* constants above)
        let mem_attrs: u64 = 0xff as u64 |           /* normal memory */
                            (0x44 as u64) << 8 |    /* normal non-cache */
                            (0x0  as u64) << 16 |   /* device nGnRnE*/
                            (0x4  as u64) << 24;    /* device nGnRE */
        unsafe {
            asm!("msr MAIR_EL1, {0}", in(reg)mem_attrs);
        }
        
        // Translation Control Register (TCR_EL1): 0x1801C321C
        // KERN: 0000_0000_0000_0000 to 0000_000F_FFFF_FFFF virt. address-space 
        //       pointed to by TTBR0_EL1 (3 level paging, 64GB)
        // USER: FFFF_0000_0000_0000 to FFFF_000F_FFFF_FFFF virt. address-space
        //       pointed to by TTBR1_EL1 (3-level paging, 64GB) 
        // that uses 4KB pages.
        // [59]    : DS=0
        // [34..32]: IPS=0001b Physical Address Size of 64GB
        // [31..30]: TG1=10b   TTBR1_EL1 Granule Size is 4KB
        // [23]    : EPD1=0b   TTBR1_EL1 Translation Disabled = False
        // [21..16]: T1SZ=28   TTBR1_EL1 Region size (2^(64-T1SZ)=2^36)
        // [15..14]: TG0=0b    TTBR0_EL1 Granule Size is 4KB
        // [13..12]: SH0=3     TTBR0_EL1 translation tables are Inner Sharable
        // [11..10]: ORGN0=0b  TTBR0_EL1 Normal memory, Outer Non-cacheable
        // [9..8]  : IRGN0=10b TTBR0_EL1 Normal memory, Inner Wrrite-Through
        // [7]     : EPD0=0b   TTBR0_EL1 Translation Disabled = False
        // [ 5.. 0]: T0SZ=28   TTBR0_EL1 Region size (2^(64-T0SZ)=2^36)
        unsafe {
            asm!("msr TCR_EL1, {0}", in(reg)0x1801C321C as u64);
        }

        // Kernel's paging structure is a level-1 table with 8 1GB
        // blocks that maps the first 4GBs of the physical memory twice:
        // Virtual Address       --> Physical Address (Mode)
        // 0   - 4GB             --> 0 - 4GB R/W KERNEL MODE
        // 4GB - 8GB             --> 0 - 4GB but as DMA memory
        //
        // Make ttbr0_el1 point to that structure.
        let kernel_lvl1_pgtbl: u64;
        unsafe{
            kernel_lvl1_pgtbl = &_KLVL1_PGTBL as *const usize as u64;
            asm!("msr TTBR0_EL1, {0}", in(reg)kernel_lvl1_pgtbl);
        }

        // Enable the MMU by modifying SCTLR_EL1, System Control Register (EL1)
        // [0]    : 
        let mut sys_ctrl: u64;
        unsafe {
            asm!("mrs {0}, SCTLR_EL1", out(reg)sys_ctrl);
        }
        sys_ctrl &= !(1<<25); // Clear EE  (BigEndian Explicit Data Access @EL1)
        sys_ctrl &= !(1<<24); // Clear E0E (BigEndian Explicit Data Access @EL0)
        //sys_ctrl &= !(1<<1);  // No alignment check
        sys_ctrl |= 1;        // Set M EL1&0 stage 1 address translation enabled
        unsafe {
            asm!(
                "msr SCTLR_EL1, {0}",
                "isb",
                in(reg)sys_ctrl
            );
        }
        dbg!("CPU{} SCTRL_EL1: {:X}\n", cpu_id(), sys_ctrl);
    }

    // Creates the initial paging structures for the process.
    // The Kernel map is already set up.
    pub fn init(&mut self) {
        self.ttbr1 = palloc().expect("Out of memory") as u64;
        unsafe {
            (self.ttbr1  as *mut u8).write_bytes(0, 0x1000);
        }
        self.tlb_page_count += 1;
    }

    //
    // Paging structure management methods 
    //
    pub fn addr_to_page_index(addr: usize) -> usize {
        addr >> 12
    }

    // Maps a page to a frame in User Address Space (addr >= MIN_VIRTUAL)
    // The assumption is that the caller has already reserved page_cnt frames
    // starting from phys_address from the physical memory manager.
    pub fn map_pages(&mut self, virt_addr: usize, phys_addr: usize, page_cnt: usize,
                     privileged: bool, writeable: bool, _executable: bool,
                    _caching: MmuCachingPolicy) -> bool {
        if virt_addr < Self::MIN_VIRTUAL as usize || 
                virt_addr > Self::MAX_VIRTUAL as usize {
            return false;
        }
        // Page-align the given addresses
        // The upper paging structures (ttbr1_el1) start indexing virtual
        // memory from MIN_VIRTUAL, so vaddr should be shifted by that amount
        let mut vaddr : u64 = (virt_addr as u64 - Self::MIN_VIRTUAL) & 
                                                        Self::PAGE_BASE_MASK;
        // Physical address is indexed from 0, so no shifting required
        let mut paddr : u64 = phys_addr as u64 & Self::PAGE_BASE_MASK;
        dbg!("map_pages(v:{:X}, p:{:X}, cnt:{}) ttbr1={:X}\n",
            vaddr, paddr, page_cnt, self.ttbr1);
        

        let l1_tbl_base = self.ttbr1 as usize;
        for _i in 0..page_cnt {
            let l1e_i = (vaddr as usize >> 30) & 0x1FF;
            let l2e_i = (vaddr as usize >> 21) & 0x1FF;
            let l3e_i = (vaddr as usize >> 12) & 0x1FF;

            let l2_tbl_base: usize;
            let l3_tbl_base: usize;

            // Level 1 - 1~GB Regions
            let mut l1e = Self::read_table_entry(l1_tbl_base, l1e_i);
            if l1e == 0 {
                // Allocate a level-2 table for this 1GB region
                l2_tbl_base = palloc().expect("Out of memory");
                self.tlb_page_count += 1;
                unsafe {
                    (l2_tbl_base as *mut u8).write_bytes(0, 0x1000);
                }
                l1e = l2_tbl_base as u64 | Self::PGENT_TBL_DESC |
                        Self::PGENT_USERMODE | Self::PGENT_INSH |
                        Self::PGENT_ACCESS;
                Self::write_table_entry(l1_tbl_base, l1e_i, l1e);
            } else {
                // Retrieve the level-2 table base from the level-1 entry
                l2_tbl_base = (l1e & Self::PGENT_PHYS_ADDR_MASK) as usize;
            }

            // Level 2 - 2~MB Regions
            let mut l2e = Self::read_table_entry(l2_tbl_base, l2e_i);
            if l2e == 0 {
                // Allocate a level-3 table for this 1GB region
                l3_tbl_base = palloc().expect("Out of memory");
                self.tlb_page_count += 1;
                unsafe {
                    (l3_tbl_base as *mut u8).write_bytes(0, 0x1000);
                }
                l2e = l3_tbl_base as u64 | Self::PGENT_TBL_DESC |
                        Self::PGENT_USERMODE | Self::PGENT_INSH |
                        Self::PGENT_ACCESS;
                Self::write_table_entry(l2_tbl_base, l2e_i, l2e);
            } else {
                // Retrieve the level-3 table base from the level-2 entry
                l3_tbl_base = (l2e & Self::PGENT_PHYS_ADDR_MASK) as usize;
            }

            // Level 3 - 4K Pages
            let mut l3e = Self::read_table_entry(l3_tbl_base, l3e_i);
            if l3e & 0x1 != 0 {
                klog!("map_pages - Virtual address {:X} is already mapped!\n",
                    virt_addr);
                return false;
            }
            l3e = (paddr & Self::PGENT_PHYS_ADDR_MASK) as u64 |
                    Self::PGENT_PG_DESC |
                    Self::PGENT_INSH | Self::PGENT_ACCESS;
            if writeable == false {
                l3e |= Self::PGENT_RO;
            }
            if privileged == false {
                l3e |= Self::PGENT_USERMODE;
            }
            Self::write_table_entry(l3_tbl_base, l3e_i, l3e);
            
            // Map the next page
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
        let vaddr_upper = virt_addr - Self::MIN_VIRTUAL as usize;
        let l1e_i = (vaddr_upper >> 30) & 0x1FF; // Index in PDPT[0]
        let l2e_i = (vaddr_upper >> 21) & 0x1FF; // Index in PD
        let l3e_i = (vaddr_upper >> 12) & 0x1FF; // Index in PT

        if self.ttbr1 == 0 {
            return None;
        }
        let l2_base = Self::read_table_entry(self.ttbr1 as usize, l1e_i)
                                                & Self::PGENT_PHYS_ADDR_MASK;
        if l2_base == 0 {
            return None;
        }
        let l3_base = Self::read_table_entry(l2_base as usize, l2e_i)
                                                & Self::PGENT_PHYS_ADDR_MASK;
        if l3_base == 0 {
            return None;
        }
        let pg_entry = Self::read_table_entry(l3_base as usize, l3e_i);
        if pg_entry & Self::PGENT_PG_DESC == 0 {
            return None;
        }
                                                
        // Clear the page entry to unmap the page
        Self::write_table_entry(l3_base as usize, l3e_i, 0);
        
        self.mapped_pages_count -= 1;
        Some((pg_entry & Self::PGENT_PHYS_ADDR_MASK) as usize)
    }

    /// Unmaps the given virtual address range and returns the number of pages
    /// that were unmapped. The caller is expected to free the physical pages
    /// after unmapping.
    pub fn unmap_pages(&mut self, virt_addr: usize, num_pages: usize) -> usize {
        let mut unmapped_pages = 0;
        for i in 0..num_pages {
            if self.unmap_page(virt_addr + i * Self::PAGE_SIZE).is_some() {
                unmapped_pages += 1;
            }
        }
        unmapped_pages
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
        dbg!("Moving to EL0 with SP: KERN={:X}, USER={:X}, EXIT_FN: {:p}={:X}\n",
                            crate::arch::aarch64_stack_pointer(), user_stack,
                            exit_handler, exit_handler as usize);
        /* Switch from EL1 to EL0
         * EL0 uses its own stack pointer (SP_EL0), so it should be set here
         * EL1 continues to use the stack allocated to the initial kernel task
         *     that's now making the jump to the user space.
         * Since the context switch logic uses the kernel stack for context
         * information, SP_EL1 also has to be updated here.
         */
        unsafe {
            asm!(
                "msr DAIFSET, #0b1111",    // Disable interrupts
                "isb",                     // clear pipeline
                "tlbi   vmalle1",         // invalidate all TLB entries
                "dsb    ish",             // ensure completion of TLB invalidatation
                "isb",                     // clear pipeline after TLB invalidation
                "msr    ttbr1_el1, {ttbr1_val}", // Switch to the new page table for user-space
                "msr    spsr_el1, xzr",    // Jump to EL0 and re-enable the interrupts upon ERET
                "msr    elr_el1, {entry_point}",
                "msr    sp_el0, {user_stack}",
                "mov    x0, {ep_arg}",
                "mov    x30, {ret_addr}",
                "eret",
                ttbr1_val   = in(reg)priv_data,
                entry_point = in(reg)entry_point,
                user_stack  = in(reg)user_stack,
                ep_arg      = in(reg)arg,
                ret_addr    = in(reg)exit_handler,
            );
        }
        panic!("Must have been unreachable!\n");
    }

    pub fn copy_priv_data(&self) -> usize {
        self.ttbr1 as usize
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
        let vaddr_upper = vaddr - Self::MIN_VIRTUAL as usize;
        let l1e_i = (vaddr_upper >> 30) & 0x1FF; // Index in PDPT[0]
        let l2e_i = (vaddr_upper >> 21) & 0x1FF; // Index in PD
        let l3e_i = (vaddr_upper >> 12) & 0x1FF; // Index in PT

        klog!("vaddr_upper = {:X}, l1e_i:{}, l1e_i:{}, l1e_i:{} - ttbr1={:X}\n",
            vaddr_upper, l1e_i, l2e_i, l3e_i, self.ttbr1);

        if self.ttbr1 == 0 {
            return;
        }
        let l2_base = Self::read_table_entry(self.ttbr1 as usize, l1e_i)
                                                & Self::PGENT_PHYS_ADDR_MASK;
        if l2_base == 0 {
            klog!("L2 table not found!\n");
            return;
        }
        let l3_base = Self::read_table_entry(l2_base as usize, l2e_i)
                                                & Self::PGENT_PHYS_ADDR_MASK;
        if l3_base == 0 {
            klog!("L3 table not found!\n");
            return;
        }
        let pg_base = Self::read_table_entry(l3_base as usize, l3e_i)
                                                & Self::PGENT_PHYS_ADDR_MASK;
        if pg_base == 0 {
            klog!("Page not found!\n");
            return;
        }
                                                
        klog!("L1_Tbl @ {:X}, L1_Tbl[{}] = {:X}\n", self.ttbr1, l1e_i,
                            Self::read_table_entry(self.ttbr1 as usize, l1e_i));
        
        klog!("  L2_Tbl @ {:X} L2_Tbl[{}] = {:X}\n", l2_base, l2e_i,
                            Self::read_table_entry(l2_base as usize, l2e_i));

        klog!("    L3_Tbl @ {:X} L3_Tbl[{}]=> {:X}\n", l3_base, l3e_i,
                            Self::read_table_entry(l3_base as usize, l3e_i));
        klog!("VADDR {:X} --> PADDR {:X}\n", vaddr, pg_base);
    }
}

impl Drop for MMUMapping {
    fn drop(&mut self) {
        let mut _pg_cnt = 0;
        let mut _pg_st_count = 0;
        // Free the level-3 tables
        let l1_tbl_base = self.ttbr1 as usize;
        for l1e_i in 0..512 {
            let l1e = Self::read_table_entry(l1_tbl_base, l1e_i);
            if l1e & Self::PGENT_TBL_DESC == 0 {
                continue;
            }
            let l2_tbl_base = (l1e & Self::PGENT_PHYS_ADDR_MASK) as usize;
            for l2e_i in 0..512 {
                let l2e = Self::read_table_entry(l2_tbl_base, l2e_i);
                if l2e & Self::PGENT_TBL_DESC == 0 {
                    continue;
                }
                let l3_tbl_base = (l2e & Self::PGENT_PHYS_ADDR_MASK) as usize;
                for l3e_i in 0..512 {
                    let l3e = Self::read_table_entry(l3_tbl_base, l3e_i);
                    if l3e & Self::PGENT_PG_DESC == 0 {
                        continue;
                    }
                    _pg_cnt += 1;
                    pfree((l3e & Self::PGENT_PHYS_ADDR_MASK) as usize);
                }
                _pg_st_count += 1;
                pfree(l3_tbl_base as usize);
            }
            _pg_st_count += 1;
            pfree(l2_tbl_base as usize);
        }
        _pg_st_count += 1;
        pfree(self.ttbr1 as usize);
        dbg!("Released {} user frames and {} paging structure frames - \
              Free frames: {}\n", _pg_cnt, _pg_st_count,
              pmm_num_free_frames());
        // Invalidate local TLB to make sure no stale entries are lef
        unsafe {
            asm!(
                "tlbi	vmalle1",
                "dsb	nsh"
            );
        }
    }
}

unsafe extern "C" {
    static _KLVL1_PGTBL: usize;
}

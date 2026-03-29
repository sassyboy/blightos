//
// BlightOS Heap Allocator
//
// Heap allocator implementation (Kalloc)
//
// Overview
// - Two-tier allocator chosen by request size:
//     1) Small allocations (<= 4032 bytes): simplified two-level segregated fit
//          implemented as a slab-like allocator with local 64-bit bitmaps per
//          cluster (fast-path TLSF-like behavior).
//     2) Large allocations (> 4032 bytes): allocate whole pages from the
//          physical page manager (PMM) via palloc_continuous / pfree_continuous
//          (page-aligned fallback).
//
// Allocation units (AU)
// - AU sizes are multiples of 64 bytes: 64 * i, where 1 <= i <= 63 (64..=4032).
// - au_index = au / 64. Index 0 is unused for simpler indexing; KALLOC_ROOT has
//     64 entries (indices 0..63), so au_index belongs to [1,63].
//
// Clusters
// - A "cluster" is a contiguous allocation of i physical 4KiB pages
//     (i = au_index).
// - Each cluster therefore has size = au_index * 4096 bytes and always contains
//     exactly 64 AUs of the corresponding AU size.
//     E.g., AU=128 -> au_index=2 -> cluster is 2 pages (8192 bytes) -> 64 * 128
//
// Cluster descriptors & descriptor pages
// - Each cluster is described by a KallocCD:
//         { base_addr: u64, au_bitmap: u64 }
//     - base_addr == 0 => cluster not allocated yet.
//     - au_bitmap uses 1 bits to denote free AUs and 0 bits for allocated AUs.
//         Newly initialized descriptors start with au_bitmap 0xFFFFFFFFFFFFFFFF
//         (all 64 AUs free). Allocation clears a bit; free sets a bit.
//     - The lowest-order set bit is found with trailing_zeros() to choose the
//         first free AU (best-fit within the cluster).
//
// - Descriptor pages (KallocCDPage):
//     - Each 4KiB descriptor page stores DESCRIPTOR_COUNT = 255 KallocCD
//         entries plus two chaining words: prev and next.
//     - The prev and next words encode two pieces of information by exploiting
//         4KiB alignment:
//         - High bytes (all bits except low 8) store the pointer to the
//             previous / next descriptor page (page-aligned).
//         - Low byte of prev stores the index (0..254) of the first descriptor
//             in the page with a free AU (free_desc_index).
//         - Low byte of next stores the number of descriptors in the page that
//             have at least one free AU (free_desc_count). A value of 0 means
//             "no free descriptors in this page". Pages are initialized with
//             next low byte = 0xFF (255 free descriptors).
//     - The encoding allows quick selection of a descriptor page that has free
//         AUs and quick access to a likely free descriptor index without
//         scanning all descriptors in the page.
//
// Root directory
// - KALLOC_ROOT is a Spinlock-protected array of 64 pointers (1 per AU class).
// - Each entry points to the head of a chain of descriptor pages for that AU
//     size class. The head is updated when new descriptor pages are allocated.
//
// Allocation flow (small requests)
// 1) Round up requested size to next multiple of 64 -> au, au_index = au/64.
// 2) Lock root and consult KALLOC_ROOT[au_index].
// 3) Walk descriptor-page chain until a page with free_desc_count > 0 is found.
//      If none exists, allocate and initialize a new descriptor page and insert
//      it at the head of the list.
// 4) From the chosen descriptor page use its free_desc_index (low byte of prev)
//      to pick a descriptor; if descriptor.base_addr == 0 allocate a new
//      cluster via palloc_continuous(au_index) and store base_addr.
// 5) Find the least-significant set bit in au_bitmap (first free AU), compute
//      object offset = bit_index * AU (AU = au_index * 64 bytes) and clear that
//      bit to mark allocation.
// 6) If clearing that bit makes the descriptor full (au_bitmap == 0), decrement
//      the page's free_desc_count (low byte of next) and, if more free
//      descriptors remain, scan descriptors in the page to update
//      free_desc_index.
//
// Free flow (small requests)
// - Given a pointer, the allocator scans descriptor pages for the AU class and
//     their descriptors until it finds the cluster whose base_addr range contains
//     the pointer. For that descriptor:
//     - Compute bit_index = (ptr - base_addr) / AU and set the corresponding bit
//         in au_bitmap to mark the AU free.
//     - If the descriptor becomes completely free (au_bitmap == all 1s) free
//         the cluster pages back to the PMM via pfree_continuous and set
//         base_addr = 0.
//     - Update the page's free_desc_count and free_desc_index appropriately:
//         - If descriptor was previously full (au_bitmap == 0 before free),
//             increment free_desc_count and, if the previous count was zero,
//             set free_desc_index to this descriptor index.
//         - Otherwise, if this descriptor index is less than the currently
//             stored free_desc_index, update free_desc_index to this index.
//     - Note that a cluster descriptor page that becomes completely free
//         (free_desc_count == 0xFF) is not freed back to the PMM for
//         performance reasons; it can be reused for future allocations in the
//         AU class.
//
// Large allocations
// - For requests that round up to au_index >= KALLOC_ROOT_ENTRIES (i.e. >4032)
//     the allocator bypasses TLSF and allocates contiguous pages from the PMM:
//     page_count = round_up(size, 4096) / 4096; palloc_continuous(page_count).
//
// Concurrency & invariants
// - KALLOC_ROOT is protected by a Spinlock during alloc/free entry points to
//     synchronize access to the root pointers and to ensure safe traversal /
//     insertion of descriptor pages. Descriptor pages and cluster descriptors
//     are accessed and mutated under that lock in the current design.
// - Descriptor pages are page-aligned, permitting the multiplexing of pointer
//     and small metadata into prev/next fields by using the low byte.
//
// Statistics & bookkeeping
// - KALLOC_DESC_PAGES and KALLOC_CLUSTER_PAGES are AtomicUsize counters keeping
//     track of descriptor pages and cluster (user-data) pages currently in use.
// - The allocator panics on out-of-memory while allocating descriptor pages or
//     clusters (explicit palloc failures).
//
// Notes and complexity
// - Allocation for small objects is fast: it avoids per-object metadata by
//     using bitmaps and uses per-page hints (free_desc_index/count) to avoid
//     scanning all descriptors on every allocation. When a cluster gets fully
//     allocated, a scan of up to 255 descriptors is performed to find the next
//     free descriptor in the cluster descriptor page.
// - Free requires a search for the descriptor that owns the pointer; worst-case
//     cost is linear in number of descriptor pages * 255 descriptors (but pages
//     without allocated clusters are skipped early by base_addr==0 checks).
// - Bitmap convention: 1 == free, 0 == allocated. Trailing zero search picks
//     least-significant free slot.
// - This is a simplified design intended to be reasonably efficient for small
//     allocations while minimizing metadata overhead and fragmentation.
// - The heap doesn't need a large physically contiguous arena and the clusers
//   can be scattered in physical memory, however, the clusters themselves are 
//   contiguous and page-aligned.
//
#![allow(dead_code)]

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
use crate::util::*;
use crate::mem::phys::*;

#[cfg(feature="debug_heap")]
macro_rules! heapdbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[HEAP] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}
#[cfg(not(feature="debug_heap"))]
macro_rules! heapdbg {
    ($($arg:tt)*) => { };
}

unsafe impl GlobalAlloc for Kalloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut addr : *mut u8;
        addr = self.tlsf_alloc(layout);
        if addr.is_null() {
            // Request too large for our TLSF allocator -> Allocate from PMM
            let page_count =
                round_up!(layout.size(), PHY_FRAME_SIZE) / PHY_FRAME_SIZE;
            addr = match palloc_continuous(page_count) {
                Some(pbase) => pbase as *mut u8,
                None        => null_mut()
            };
            heapdbg!("PALLOC {:p} Pages = {}\n", addr, page_count);
        }
        addr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if self.tlsf_free(ptr, layout) == false {
            // Must have been allocated from PMM
            let page_count= 
                    round_up!(layout.size(), PHY_FRAME_SIZE) / PHY_FRAME_SIZE;

            heapdbg!("PFREE {:p} Pages = {}\n", ptr, page_count);
            pfree_continuous(ptr as usize, page_count);
        }
    }
}

//
// Allocation Unit (AU): 
//   Request Size Classes: 64i where i is an index into the root directory,
//   and 1 <= i <= 63, i.e, AUs are 64, 128, 192, 256, ..., 3968, 4032
//
// Allocation Policy:
//   Best fit at the first level (directory): Smallest AU size class that can
//     accommodate the requested size
//   First fit at the second level: First cluster with a free AU in the chosen
//     AU size class.
//   
// Cluster: 
//   64 continguous AUs that can be tracked via a 64-bit bitmap. 64 AUs can
//   span across multiple 4KB pages that must be contiguous.
//
//   # of 4KB-Pages per Cluster = i, where i is an index into the root directory
//   E.g., for 128-byte AUs, we allocate 2 contiguous 4K pages at a time, which
//   is 8192 bytes / 128-bytes-per-au = 64 AUs in a cluster.
//
// Root Directory:
//   One pointer per AU size class that points to a page of Cluster Descriptors.
//   Each page of Cluster descriptors contains 255 descriptors and 2 pointers
//   to chain descriptor pages together.
//   Each Cluster Descriptor points to i contiguous pages and maintains
//   [0] -> Not used for AU <--> i conversion ease
//   [1] -> [[base,bitmap], [base,bitmap], ..., [prev, next]] -> 64-B clusters
//   [2] -> [[base,bitmap], [base,bitmap], ..., [prev, next]] -> 128-B clusters
//           |------ 255 Cluster Descriptors --|
//
//   Since each cluster descriptor page is on a 4KB boundary, the lower byte
//   of prev an next can encode the following extra information to make finding
//   the next free AU easier:
//   - prev[7..0] = Index of the first cluster descriptor in the page with a
//                  free AU.
//   - next[7..0] = Number of descriptors in the page with free AUs. A value of
//                  0 means we need to follow the chain.
//

#[derive(Clone, Copy)]
struct KallocCD {
    pub base_addr:  u64,
    pub au_bitmap:  u64
}
impl KallocCD {
    pub const fn new() -> Self {
        Self {
            base_addr: 0,
            au_bitmap: 0xFFFFFFFFFFFFFFFF // Mark all AUs as free
        }
    }
}

// Each page keeps track of 16320 AUs, i.e., 255 clusters, each of whcih
// accommodates 64 AUs.
struct KallocCDPage {
    pub descriptors:    [KallocCD; Self::DESCRIPTOR_COUNT],
    // Address of the previous descriptor page for the AU size class
    // (0 if this is the first page)
    // Lower byte: index of first descriptor in the page with a free AU
    pub prev:           usize,
    // Address of the next descriptor page for the AU size class
    // Lower byte: number of descriptors in the page with free AUs
    pub next:           usize,
}
impl KallocCDPage {
    const DESCRIPTOR_COUNT: usize = 255;
    fn alloc_and_init(prev: usize) -> usize {
        match palloc() {
            Some(addr)  => {
                Self::init(addr, prev);
                KALLOC_DESC_PAGES.fetch_add(1, Ordering::Relaxed);
                addr
            },
            None        => {
                panic!("Out of memory!");
            }
        }
    }

    fn free(base_addr: usize) {
        // Manage chain pointers before freeing the page
        let prev = Self::prev(base_addr);
        let next = Self::next(base_addr);
        if prev != 0 {
            Self::set_next(prev, next);
        }
        if next != 0 {
            Self::set_prev(next, prev);
        }
        // Free the page
        pfree(base_addr);
        KALLOC_DESC_PAGES.fetch_sub(1, Ordering::Relaxed);
    }

    fn init(base_addr: usize, prev: usize) {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        // Initialize the cluster descriptors in the page
        for i in 0..Self::DESCRIPTOR_COUNT {
            dp.descriptors[i] = KallocCD::new()
        }
        // Initialize the chaining pointers
        dp.prev = prev | 0x0;   // 0: index of first free descriptor in the page
        dp.next = 0xFF;         // 255 descriptors with free AUs in the page
    }

    // Helper functions to access cluster descriptors in the page
    fn get_desc_bitmap(base_addr: usize, index: usize) -> u64 {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.descriptors[index].au_bitmap
    }

    fn get_desc_base(base_addr: usize, index: usize) -> usize {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.descriptors[index].base_addr as usize
    }

    // Helper functions for the prev chain-pointer
    fn prev(base_addr: usize) -> usize {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.prev & 0xFFFFFFFFFFFFFF00
    }
    fn free_desc_index(base_addr: usize) -> usize {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.prev & 0xFF
    }
    fn set_prev(base_addr: usize, prev: usize) {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.prev = (dp.prev & 0xFF) | (prev & 0xFFFFFFFFFFFFFF00);
    }
    fn set_free_desc_index(base_addr: usize, index: usize) {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.prev = (dp.prev & 0xFFFFFFFFFFFFFF00) | (index & 0xFF);
    }

    // Helper functions for the next chain-pointer
    fn next(base_addr: usize) -> usize {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.next & 0xFFFFFFFFFFFFFF00
    }
    fn free_desc_count(base_addr: usize) -> usize {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        (dp.next as usize) & 0xFF
    }
    fn set_next(base_addr: usize, next: usize) {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.next = (dp.next & 0xFF) | (next & 0xFFFFFFFFFFFFFF00);
    }
    fn set_free_desc_count(base_addr: usize, free_desc_count: usize) {
        let dp;
        unsafe {
            let des_pg_ptr: *mut KallocCDPage =
                base_addr as *mut KallocCDPage;
            dp = &mut (*des_pg_ptr);
        }
        dp.next = (dp.next & 0xFFFFFFFFFFFFFF00) | (free_desc_count & 0xFF);
    }
}


pub struct Kalloc {

}

const KALLOC_ROOT_ENTRIES: usize = 64;
static KALLOC_ROOT: Spinlock<[usize; KALLOC_ROOT_ENTRIES]> = 
                        Spinlock::new([0; KALLOC_ROOT_ENTRIES]);
// Statistics for debugging purposes
// 1) Total number of pages allocated for clusters and descriptor pages
static KALLOC_DESC_PAGES: AtomicUsize = AtomicUsize::new(0);
static KALLOC_CLUSTER_PAGES: AtomicUsize = AtomicUsize::new(0);

impl Kalloc {
    
    pub const fn new() -> Self {
        Self { }
    }

    unsafe fn tlsf_alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 || layout.align() > 64 {
            klog!("TLSF - Invalid allocation request: size {}, align {}\n",
                    layout.size(), layout.align());
            return null_mut();
        }

        let au      = round_up!(layout.size(), 64);
        let au_index= au / 64;
        heapdbg!("tlsf_alloc({}, {}) au:{}, au_index:{}\n",
            layout.size(), layout.align(), au, au_index);
        let mut root = KALLOC_ROOT.lock();
        if au_index < KALLOC_ROOT_ENTRIES {
            // (Optional) Allocate the first Cluster Descriptor Page for AU
            if (*root)[au_index] == 0 {
                let cdp_addr = KallocCDPage::alloc_and_init(0);
                (*root)[au_index] = cdp_addr;
            }
            // Find the location where the data should go
            return Self::cluster_search_alloc(&mut *root, au_index) as *mut u8;
        }
        null_mut() // Request not handled by my TLSF implementation
    }

    unsafe fn tlsf_free(&self, ptr: *mut u8, layout: Layout) -> bool {
        let au      = round_up!(layout.size(), 64);
        let au_index= au / 64;
        heapdbg!("tlsf_free(@{:X}, sz:{}, al:{}) au:{}, au_index:{}\n",
            ptr as usize, layout.size(), layout.align(), au, au_index);

        if au_index < KALLOC_ROOT_ENTRIES {
            let root = KALLOC_ROOT.lock();
            Self::cluster_search_free(&*root, ptr as usize, au_index);
            return true;
        }
        false // Request not handled by my TLSF implementation
    }

    // Returns the address in which the new object can be written into
    unsafe fn cluster_search_alloc(root: &mut [usize], au_index: usize) -> usize {
        // 1) Find the first cluster descriptor page that has free AU
        let mut cur_cdp_addr = root[au_index]; // This is never zero
        let mut free_desc_count: usize = 0;
        while cur_cdp_addr != 0 {
            free_desc_count = KallocCDPage::free_desc_count(cur_cdp_addr);
            if free_desc_count > 0 {
                // Found a page with a free AU
                break;
            }
            cur_cdp_addr = KallocCDPage::next(cur_cdp_addr);
        }
        if free_desc_count == 0 {
            // No page with free AU - Allocate a new cluster descriptor page and
            // add it to the chain from the head.
            let new_cdp_addr = KallocCDPage::alloc_and_init(0);
            KallocCDPage::set_next(new_cdp_addr, root[au_index]);
            KallocCDPage::set_prev(root[au_index], new_cdp_addr);
            root[au_index] = new_cdp_addr;
            cur_cdp_addr = new_cdp_addr;
            free_desc_count = KallocCDPage::free_desc_count(cur_cdp_addr);
        }

        // 2) Find the first cluster descriptor with a free AU in the page
        let cur_cdp_ptr = cur_cdp_addr as *mut KallocCDPage;
        let free_desc_index = KallocCDPage::free_desc_index(cur_cdp_addr);
        let cd = &mut (*cur_cdp_ptr).descriptors[free_desc_index];
        if cd.au_bitmap == 0 {
            panic!("Bug: free_desc_index points to a descriptor with no free \
                AU! au_index: {}, cdp_addr: {:X}\n", au_index, cur_cdp_addr);
        }

        if cd.base_addr == 0 {
            // New cluster! allocate it first
            cd.base_addr = Self::cluster_alloc(au_index) as u64;
        }
        // 3) Find the index of the first set bit int the bitmap to allocate AU
        let bit_index = cd.au_bitmap.trailing_zeros();
        if bit_index >= 64 {
            panic!("Bug: invalid bit index {} in cluster_search_alloc", bit_index);
        }
        let offset    = (bit_index as usize) * au_index * 64;
        // 4) Update the descriptor - Clear the bit for to the allocated AU
        cd.au_bitmap &= !(1u64 << bit_index);
        heapdbg!("  CLUSTER SEARCH_ALLOC: Base: {:X}, Bitmap: {:X}, Off:{:X}\n",
                cd.base_addr, cd.au_bitmap, offset);
        // 5) Update the free descriptor count in the page if this cluster is now full
        if cd.au_bitmap == 0 {
            KallocCDPage::set_free_desc_count(cur_cdp_addr, free_desc_count - 1);
            if free_desc_count > 1 {
                // Update the free descriptor index in the page if there are
                // more free descriptors
                let mut new_free_desc_index = 0;
                for i in 0..KallocCDPage::DESCRIPTOR_COUNT {
                    if KallocCDPage::get_desc_bitmap(cur_cdp_addr, i) != 0u64 {
                        new_free_desc_index = i;
                        break;
                    }
                }
                KallocCDPage::set_free_desc_index(cur_cdp_addr, new_free_desc_index);
            }
        }
        
        cd.base_addr as usize + offset
    }

    unsafe fn cluster_search_free(root: &[usize], addr: usize, au_index: usize){
        let cluster_size = au_index * PHY_FRAME_SIZE;

        // Loop over all cluster descriptors in all available cluster descriptor
        // pages for the AU size class to find the one corresponding to the
        // address to be freed. Once found, update the bitmap to mark the AU as
        // free. If the cluster becomes completely free, free the cluster and
        // Search through the chain of descriptor pages for the AU size class.
        let mut cur_cdp_addr = root[au_index];
        while cur_cdp_addr != 0 {
            let des_pg_ptr = cur_cdp_addr as *mut KallocCDPage;
            for i in 0..KallocCDPage::DESCRIPTOR_COUNT {
                let cd = unsafe { &mut (*des_pg_ptr).descriptors[i] };
                if cd.base_addr == 0 {
                    // Empty descriptor - No cluster allocated yet, so skip
                    continue;
                }
                if addr < cd.base_addr as usize ||
                    addr >=  (cd.base_addr as usize + cluster_size) {
                    // Address doesn't belong to this cluster, so skip
                    continue;
                }
                // Found the cluster descriptor for this address
                let offset = addr - cd.base_addr as usize;
                let bit_index = offset / (au_index * 64);
                if bit_index >= 64 {
                    klog!("BUG: CLUSTER_SEARCH_FREE bad bit_index {} for \
                            addr {:X}\n", bit_index, addr);
                    return;
                }
                let cd_was_full = cd.au_bitmap == 0;
                // Mark AU as free
                cd.au_bitmap |= 1u64 << (bit_index as u32);
                heapdbg!("  CLUSTER SEARCH_FREE: Base: {:X}, Bitmap: {:X}, \
                        Off:{:X}\n", cd.base_addr, cd.au_bitmap, offset);
                if cd.au_bitmap == 0xFFFFFFFFFFFFFFFF {
                    // Entire cluster is free -> return pages to PMM and clear base
                    Self::cluster_free(cd.base_addr as usize, au_index);
                    cd.base_addr = 0;
                }
                // Update page metadata: free descriptor count/index
                let cdp_free_count = KallocCDPage::free_desc_count(cur_cdp_addr);
                if cd_was_full {
                    // Descriptor was full before; now has at least one free AU
                    // So the page's free count should go up by 1
                    KallocCDPage::set_free_desc_count(cur_cdp_addr,
                                                        cdp_free_count + 1);
                    // Edge-case: If a page was completely full before this free
                    // operation, then we need to update the free descriptor
                    // regardless of the last free_desc_index because it would
                    // be invalid (pointing to a descriptor with no free AU)
                    if cdp_free_count == 0 {
                        KallocCDPage::set_free_desc_index(cur_cdp_addr, i);
                    }
                } else {
                    // Ensure free descriptor index is updated if this
                    // descriptor is before the currently known free descriptor
                    // index
                    let cur_idx = KallocCDPage::free_desc_index(cur_cdp_addr);
                    if i < cur_idx {
                        KallocCDPage::set_free_desc_index(cur_cdp_addr, i);
                    }
                }
                return;
            }
            cur_cdp_addr = KallocCDPage::next(cur_cdp_addr);
        }

        // If we reach here the descriptor wasn't found in the chain
        klog!("BUG: CLUSTER_SEARCH_FREE: Addr {:X} (au_inx: {}) not found\n",
            addr, au_index);
    }

    // Cluster allocation and free functions
    fn cluster_alloc(au_index: usize) -> usize {
        match palloc_continuous(au_index) {
            Some(addr)      => {
                heapdbg!("    CLUSTER ALLOC: base_addr: {:X}, au_index/pages: {}\n",
                    addr, au_index);
                KALLOC_CLUSTER_PAGES.fetch_add(au_index, Ordering::Relaxed);
                addr
            },
            None            => {panic!("Out of memory!")}
        }
    }

    fn cluster_free(base_addr: usize, au_index: usize) {
        heapdbg!("    CLUSTER FREE:  base_addr: {:X}, au_index/pages: {}\n",
                    base_addr, au_index);
        KALLOC_CLUSTER_PAGES.fetch_sub(au_index, Ordering::Relaxed);
        pfree_continuous(base_addr, au_index);
    }

    // Helper functions for statistics/debugging purposes
    pub fn metadata_pages_used() -> usize {
        KALLOC_DESC_PAGES.load(Ordering::Relaxed)
    }
    pub fn userdata_pages_used() -> usize {
        KALLOC_CLUSTER_PAGES.load(Ordering::Relaxed)
    }

}



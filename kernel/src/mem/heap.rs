//
// Heap Allocator
//
// Chosen based on request size:
// 1) Size <= 4032 -> A simplified 2-level segregated fit allocation
//                    Level-1 A directory of cluster-descriptor-lists
//                    Level-2 Cluster descriptor list, wherein each descriptor
//                            contains a bitmap of allocated units within each
//                            cluster, and the base address of that cluster
//                    NB: A cluster is a number of contiguous 4KB pages capable
//                        of storing 64 objects of a specific size (AU).
// 2) Size >  4032 -> Round-up(Size,4096)/4096 contiguous pages from PMM/VMM
//
// 

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
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

static HEAP_LOCK: Spinlock<usize> = Spinlock::new(0);

unsafe impl GlobalAlloc for Kalloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let lock = HEAP_LOCK.lock();
        let mut addr : *mut u8;
        addr = self.tlsf_alloc(layout);
        if addr == null_mut() {
            // Request too large for our TLSF allocator -> Allocate from PMM
            let page_count =
                round_up!(layout.size(), PHY_FRAME_SIZE) / PHY_FRAME_SIZE;
            addr = match palloc_continuous(page_count) {
                Some(pbase) => pbase as *mut u8,
                None        => null_mut()
            };
            heapdbg!("PALLOC {:p} Pages = {}\n", addr, page_count);
        }
        drop(lock);
        addr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let lock = HEAP_LOCK.lock();
        if self.tlsf_free(ptr, layout) == false {
            // Must have been allocated from PMM
            let page_count= 
                    round_up!(layout.size(), PHY_FRAME_SIZE) / PHY_FRAME_SIZE;

            heapdbg!("PFREE {:p} Pages = {}\n", ptr, page_count);
            pfree_continuous(ptr as usize, page_count);
        }
        drop(lock);
    }
}

//
// A simplified Two-Level Segregated Fit Allocation
// More like a slab allocator + Local bitmaps
//
// Allocation Unit (AU): 
//   Request Size Classes: 64i where i is an index into the root directory,
//   and 1 <= i <= 63, i.e, AUs are 64, 128, 192, 256, ..., 3968, 4032
//
// Allocation Policy:
//   Best fit - Smallest AU that can accommodate the requested size
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
//   [1] -> [[base,bitmap], [base,bitmap], ..., [prev, next]] -> 64-bit clusters
//   [2] -> [[base,bitmap], [base,bitmap], ..., [prev, next]] -> 128-but clust.
//           |------ 255 Cluster Descriptors --|


#[derive(Clone, Copy)]
struct KallocClusterDescriptor {
    pub base_addr:  u64,
    pub au_bitmap:  u64
}
impl KallocClusterDescriptor {
    pub const fn new() -> Self {
        Self {
            base_addr: 0,
            au_bitmap: 0xFFFFFFFFFFFFFFFF // Mark all AUs as free
        }
    }
}

struct KallocClusterDescriptorPage {
    pub descriptors: [KallocClusterDescriptor; 255],
    pub _prev:       u64,
    pub _next:       u64
}
impl KallocClusterDescriptorPage {
    pub const fn new() -> Self {
        Self {
            descriptors:[KallocClusterDescriptor::new(); 255],
            _prev: 0,
            _next: 0
        }
    }
}

pub struct Kalloc {}

const KALLOC_ROOT_ENTRIES: usize = 64;
static mut KALLOC_ROOT: [usize; KALLOC_ROOT_ENTRIES] = [0; KALLOC_ROOT_ENTRIES];
impl Kalloc {
    
    pub const fn new() -> Self {
        Self { }
    }

    unsafe fn tlsf_alloc(&self, layout: Layout) -> *mut u8 {
        let au      = round_up!(layout.size(), 64);
        let au_index= au / 64;
        heapdbg!("tlsf_alloc({}, {}) au:{}, au_index:{}\n",
            layout.size(), layout.align(), au, au_index);

        if au_index < KALLOC_ROOT_ENTRIES {
            // (Optional) Allocate the first Cluster Descriptor Page for AU
            if KALLOC_ROOT[au_index] == 0 {
                match palloc(){
                    Some(cdp_base)  => {
                        Self::desc_page_init(cdp_base);
                        KALLOC_ROOT[au_index] = cdp_base;
                    }
                    None            => {
                        return null_mut();
                    }
                }
            }
            // Find the location where the data should go
            return Self::cluster_search_alloc(au_index) as *mut u8;
        }
        null_mut() // Request not handled by my TLSF implementation
    }

    unsafe fn tlsf_free(&self, ptr: *mut u8, layout: Layout) -> bool {
        let au      = round_up!(layout.size(), 64);
        let au_index= au / 64;
        heapdbg!("tlsf_free(@{:X}, sz:{}, al:{}) au:{}, au_index:{}\n",
            ptr as usize, layout.size(), layout.align(), au, au_index);

        if au_index < KALLOC_ROOT_ENTRIES {
            Self::cluster_search_free(ptr as usize, au_index);
            return true;
        }
        false // Request not handled by my TLSF implementation
    }

    unsafe fn desc_page_init(base_addr: usize) {
        let des_pg_ptr: *mut KallocClusterDescriptorPage =
            base_addr as *mut KallocClusterDescriptorPage;
        *des_pg_ptr = KallocClusterDescriptorPage::new();
    }
    fn cluster_alloc(au_index: usize) -> usize {
        match palloc_continuous(au_index) {
            Some(addr)      => {
                heapdbg!("    CLUSTER ALLOC: base_addr: {:X}, au_index/pages: {}\n",
                    addr, au_index);
                addr
            },
            None            => {panic!("Out of memory!")}
        }
    }

    fn cluster_free(base_addr: usize, au_index: usize) {
        heapdbg!("    CLUSTER FREE:  base_addr: {:X}, au_index/pages: {}\n",
                    base_addr, au_index);
        pfree_continuous(base_addr, au_index);
    }

    // Returns the address in which the new object can be written into
    unsafe fn cluster_search_alloc(au_index: usize) -> usize {
        // TODO: Chain more CDPs if this one is out of space
        let des_pg_ptr: *mut KallocClusterDescriptorPage = 
                KALLOC_ROOT[au_index] as *mut KallocClusterDescriptorPage;

        for cd in &mut (*des_pg_ptr).descriptors {
            if cd.au_bitmap > 0 {
                // There is room in this cluster
                if cd.base_addr == 0 {
                    // New cluster! allocate it first
                    cd.base_addr = Self::cluster_alloc(au_index) as u64;
                }
                // Find the index of the first set bit int the bitmap
                let bit_index = cd.au_bitmap.trailing_zeros();
                let offset    = bit_index as usize * au_index * 64;
                // Clear the bit
                cd.au_bitmap &= !(1 << bit_index);
                heapdbg!("  CLUSTER SEARCH_ALLOC: Base: {:X}, Bitmap: {:X}, Off:{:X}\n",
                        cd.base_addr, cd.au_bitmap, offset);
                return cd.base_addr as usize + offset;
            }
        }
        0
    }

    unsafe fn cluster_search_free(addr: usize, au_index: usize) {
        let cluster_size = au_index * PHY_FRAME_SIZE;
        let des_pg_ptr: *mut KallocClusterDescriptorPage = 
                KALLOC_ROOT[au_index] as *mut KallocClusterDescriptorPage;

        for cd in &mut (*des_pg_ptr).descriptors {
            if cd.base_addr == 0 {
                klog!("BUG: CLUSTER_SEARCH_FREE: Addr {:X} (au_inx: {}) not found\n",
                    addr, au_index);
                return;
            }
            if  addr >= cd.base_addr as usize &&
                addr <= (cd.base_addr as usize + cluster_size) {
                // Object contain in this cluster
                let offset = addr - cd.base_addr as usize;
                let bit_index = offset / (au_index * 64);
                cd.au_bitmap |= 1 << bit_index;
                heapdbg!("  CLUSTER_SEARCH_FREE: Base: {:X}, Bitmap: {:X}, Off:{:X}\n",
                        cd.base_addr, cd.au_bitmap, offset);
                if cd.au_bitmap == 0xFFFFFFFFFFFFFFFF {
                    // This cluster is empty - return the allocation
                    Self::cluster_free(cd.base_addr as usize, au_index);
                    cd.base_addr = 0;
                }
                return;
            }
        }
        // TODO: Follow the chain
    }

}



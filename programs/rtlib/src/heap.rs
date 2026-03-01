//
// BlightOS Heap Allocator
// Simple free list allocator with coalescing. Not thread safe yet
//

use core::alloc::{GlobalAlloc, Layout};
use core::mem::size_of;
use core::ptr;
use crate::*;
use crate::syscall::{Syscall, ProcCtlOpCode, ProcCtlResizeHeapArgs};
use crate::task::*;

///
/// Global Heap Allocator that plugs into Rust's alloc interface
/// 
pub struct Malloc{}

#[global_allocator]
static ALLOCATOR: Malloc = Malloc {};
static FFH : Spinlock<FirstFitHeap> = Spinlock::new(FirstFitHeap::new());

unsafe impl GlobalAlloc for Malloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut heap = FFH.lock();
        heap.alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        let mut heap = FFH.lock();
        heap.free(ptr);
    }
}

impl Malloc {
    ///
    /// Optional initialization function to be called by the application at
    /// startup. The heap will be lazily initialized on first allocation if this
    /// is not called.
    pub fn init() {
        let mut heap = FFH.lock();
        heap.init();
    }
    pub fn release_unused_memory() -> usize {
        let mut heap = FFH.lock();
        heap.release_unused_memory()   
    }
    pub fn heap_base() -> usize {
        let heap = FFH.lock();
        heap.heap_base
    }
    pub fn heap_size() -> usize {
        let heap = FFH.lock();
        heap.heap_size 
    }
}

///
/// A simple first-fit heap allocator with coalescing. Not thread safe per se
/// 
struct FirstFitHeap {
    heap_base:      usize,
    heap_size:      usize,
    free_list_head: usize,
}

impl FirstFitHeap {
    // Block format (usize is 8 bytes on 64-bit):
    // 1) Allocated bock
    //    offset    data
    //      0       [ header: size | alloc bit ]
    //      8       [ payload... ]
    // 2) Free block
    //    offset    data
    //      0       [ header: size | alloc bit ]
    //      8       [ next:   offset of next free block ]
    //      16+     [ unused... ]
    //
    const ALLOC_BIT: usize = 1;
    const NULL: usize = usize::MAX;
    /// Header stores size with low bit = allocated flag.
    /// Actual usable size = size & !1
    const HEADER_SIZE: usize = size_of::<usize>();
    // A block has at a header and a footer (next pointer when free) at least
    const MIN_BLOCK_SIZE: usize = 2 * Self::HEADER_SIZE;

    const INIT_HEAP_SIZE: usize = 1024 * 256;   // 256 KiB
    const HEAP_GROW_SIZE: usize = 1024 * 256;   // 256 KiB
    const fn new() -> Self {
        Self {
            heap_base: 0,
            heap_size: 0,
            free_list_head: Self::NULL,
        }
    }

    /// Initialize heap. Call once before allocation.
    fn init(&mut self) {
        // Request initial heap memory from kernel via syscall
        if let Some(args) = self.resize_heap(Self::INIT_HEAP_SIZE as isize) {
            self.heap_base = args.heap_base;
            self.heap_size = args.heap_size;
        } else {
            panic!("Failed to initialize heap");
        }
        // single free block covering whole heap
        self.set_header(0, self.heap_size, false);
        self.set_free_next(0, Self::NULL);
        self.free_list_head = 0;
    }
    /// Allocate size bytes, returns pointer into HEAP or null.
    pub unsafe fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        if self.free_list_head == Self::NULL {
            self.init();
        }
        if size == 0 {
            return ptr::null_mut();
        }
        // use the requested layout alignment for the payload (not the pointer size)
        let payload_align = align;
        let payload = Self::align_up(size, payload_align);
        let needed = Self::HEADER_SIZE + payload;
        let mut prev: Option<usize> = None;
        let mut cur = self.free_list_head;

        while cur != Self::NULL {
            let cur_size = self.get_block_size(cur);
            if cur_size >= needed {
                // found fit
                let remain = cur_size - needed;
                if remain >= Self::MIN_BLOCK_SIZE {
                    // split
                    let new_free_off = cur + needed;
                    self.set_header(new_free_off, remain, false);
                    let free_next = self.next_free_block(cur);
                    self.set_free_next(new_free_off, free_next);
                    self.set_header(cur, needed, true);
                    // update free list links
                    match prev {
                        Some(p) => self.set_free_next(p, new_free_off),
                        None => self.free_list_head = new_free_off,
                    }
                } else {
                    // use entire block
                    self.set_header(cur, cur_size, true);
                    match prev {
                        Some(p) => {
                            let free_next_off = self.next_free_block(cur);
                            self.set_free_next(p, free_next_off)
                        },
                        None => self.free_list_head = self.next_free_block(cur),
                    }
                }
                return (self.heap_base as *mut u8).add(cur + Self::HEADER_SIZE)
            }
            prev = Some(cur);
            cur = self.next_free_block(cur);
        }
        // out of memory - try to grow the heap by requesting the first
        // multiple of HEAP_GROW_SIZE that can fit the new block
        let grow_size = Self::align_up(needed, Self::HEAP_GROW_SIZE);
        if let Some(args) = self.resize_heap(grow_size as isize) {
            let old_heap_size = self.heap_size;
            self.heap_size = args.heap_size;
            // println!("Heap grown by {} bytes (request size: {}). \
            //         Old Size {}, New size: {}",
            //         grow_size, size, old_heap_size, self.heap_size);
            // add new free block at end of heap
            let new_block_off = old_heap_size;
            self.set_header(new_block_off, self.heap_size - old_heap_size, false);
            self.set_free_next(new_block_off, Self::NULL);
            // add to free list
            if self.free_list_head == Self::NULL {
                self.free_list_head = new_block_off;
            } else {
                let mut last = self.free_list_head;
                while self.next_free_block(last) != Self::NULL {
                    last = self.next_free_block(last);
                }
                self.set_free_next(last, new_block_off);
            }
            return self.alloc(size, align); // retry allocation after growing heap
        } else {
            return ptr::null_mut(); // failed to grow heap
        }
    }

    /// Free a previously allocated pointer.
    pub unsafe fn free(&mut self, p: *mut u8) {
        if p.is_null() {
            return;
        }

        let off = (p as usize).wrapping_sub(self.heap_base);
        if off < Self::HEADER_SIZE || off >= self.heap_size {
            return; // invalid pointer
        }
        let mut block = off - Self::HEADER_SIZE;
        if !self.is_allocated(block) {
            return; // double free or invalid
        }
        let mut size = self.get_block_size(block);
        self.set_header(block, size, false);
        // insert at head
        self.set_free_next(block, self.free_list_head);
        self.free_list_head = block;

        // try coalescing with right neighbor
        //
        // Logic: look for a free block whose offset equals the immediate right
        // neighbor of the current block (block + size). If found, merge that
        // free block into the current block by increasing the size of the
        // current block and removing the right block from the free list.
        // Repeat until there is no free block immediately to the right.
        loop {
            let mut merged = false;
            // check right neighbor: block + size
            let right = block + size;
            if right < self.heap_size {
                // search free list for a free block at offset == right
                let mut prev = None;
                let mut cur = self.free_list_head;
                while cur != Self::NULL {
                    if cur == right {
                        // merge cur into block
                        let right_size = self.get_block_size(cur);
                        size += right_size;
                        self.set_header(block, size, false);
                        // remove cur from free list
                        match prev {
                            Some(p) => {
                                let free_next_off = self.next_free_block(cur);
                                self.set_free_next(p, free_next_off)
                            },
                            None => {
                                self.free_list_head = self.next_free_block(cur);
                            }
                        }
                        merged = true;
                        break;
                    }
                    prev = Some(cur);
                    cur = self.next_free_block(cur);
                }
            }
            if !merged {
                break;
            }
        }

        // coalesce left neighbor: look for any free block whose end == block
        //
        // Logic: search the free list for a free block 'cur' whose end
        // (cur + cur_size) exactly abuts the start of 'block'. When found,
        // remove both 'cur' and the current 'block' entries from the free list,
        // merge them into a single block that starts at 'cur' and has the
        // combined size, then insert the merged block at the head of the free
        // list. Repeat until no adjacent left free block exists.
        loop {
            let mut merged = false;
            // find a free block 'cur' such that cur + cur_size == block
            let mut cur = self.free_list_head;
            while cur != Self::NULL {
                let cur_size = self.get_block_size(cur);
                if cur + cur_size == block {
                    // remove 'cur' from free list
                    let mut p = None;
                    let mut c = self.free_list_head;
                    while c != Self::NULL {
                        if c == cur {
                            match p {
                                Some(pp) => {
                                    let off = self.next_free_block(c);
                                    self.set_free_next(pp, off);
                                },
                                None => {
                                    self.free_list_head = self.next_free_block(c);
                                }
                            }
                            break;
                        }
                        p = Some(c);
                        c = self.next_free_block(c);
                    }
                    // remove 'block' from free list (it may be at head or elsewhere)
                    let mut p2 = None;
                    let mut c2 = self.free_list_head;
                    while c2 != Self::NULL {
                        if c2 == block {
                            match p2 {
                                Some(pp) => {
                                    let off = self.next_free_block(c2);
                                    self.set_free_next(pp, off);
                                },
                                None => {
                                    self.free_list_head = self.next_free_block(c2);
                                }
                            }
                            break;
                        }
                        p2 = Some(c2);
                        c2 = self.next_free_block(c2);
                    }
                    // merged block starts at 'cur'
                    block = cur;
                    size = cur_size + size;
                    self.set_header(block, size, false);
                    // insert merged block at head
                    self.set_free_next(block, self.free_list_head);
                    self.free_list_head = block;
                    merged = true;
                    break;
                }
                cur = self.next_free_block(cur);
            }
            if !merged {
                break;
            }
        }

        // coalescing complete
    }

    ///
    /// Releases any unused memory over HEAP_GROW_SIZE back to the kernel.
    /// This can be called periodically by the application (via the Malloc
    /// interface) to return memory to the system.
    /// Returns the amount of memory released back to the system.
    /// 
    pub fn release_unused_memory(&mut self) -> usize {
        // Strategy: find the largest contiguous free block at the end of the heap.
        // If it's larger than HEAP_GROW_SIZE, release the excess back to the kernel
        // by shrinking the heap via syscall. Update free list accordingly.
        println!("Attempting to release unused heap memory back to kernel. \
                    Current heap size: {} bytes", self.heap_size);
        let mut last_free = None;
        let mut cur = self.free_list_head;
        while cur != Self::NULL {
            last_free = Some(cur);
            cur = self.next_free_block(cur);
        }
        if let Some(last) = last_free {
            let last_size = self.get_block_size(last);
            let last_end = last + last_size;
            if last_end == self.heap_size && last_size > Self::HEAP_GROW_SIZE {
                // can release memory back to kernel
                let excess = last_size - Self::HEAP_GROW_SIZE;
                if let Some(args) = self.resize_heap(-(excess as isize)) {
                    self.heap_size = args.heap_size;
                    // update free block header
                    self.set_header(last, Self::HEAP_GROW_SIZE, false);
                    println!("Released {} bytes of unused heap memory back to kernel. New heap size: {} bytes",
                        excess, self.heap_size);
                    return excess;
                }
            }
        }
        0 // nothing released
    }

    fn align_up(x: usize, a: usize) -> usize {
        (x + a - 1) & !(a - 1)
    }

    fn read_usize(&self, offset: usize) -> usize {
        unsafe {
            let ptr = (self.heap_base as *const u8).add(offset) as *const usize;
            ptr::read(ptr)
        }
    }

    fn write_usize(&mut self, offset: usize, val: usize) {
        unsafe {
            let ptr = (self.heap_base as *mut u8).add(offset) as *mut usize;
            ptr::write(ptr, val);
        }
    }

    ///
    /// Returns the size of the block at offset (without allocation bit).
    /// Panics if offset is out of heap bounds.
    /// 
    fn get_block_size(&mut self, offset: usize) -> usize {
        if offset >= self.heap_size {
            panic!("Invalid block offset");
        }
        self.read_usize(offset) & !Self::ALLOC_BIT
    }

    ///
    /// Returns true if the block at offset is allocated, false if free.
    /// Panics if offset is out of heap bounds.
    /// 
    fn is_allocated(&mut self, offset: usize) -> bool {
        if offset >= self.heap_size {
            panic!("Invalid block offset");
        }
        (self.read_usize(offset) & Self::ALLOC_BIT) != 0
    }

    ///
    /// Sets the header of a block at offset with given size and allocation
    /// status. Panics if the offset falls outside the heap or if size is too large.
    /// 
    fn set_header(&mut self, offset: usize, size: usize, allocated: bool) {
        if offset >= self.heap_size || size > self.heap_size - offset {
            panic!("Invalid block header parameters");
        }
        let v = size | (if allocated { Self::ALLOC_BIT } else { 0 });
        self.write_usize(offset, v);
    }

    ///
    /// Returns the offset of the next free block in the free list after the  
    /// free block at `free_offset`.
    /// 
    fn next_free_block(&mut self, cur_free_offset: usize) -> usize {
        // next pointer stored in payload (first usize of payload)
        self.read_usize(cur_free_offset + Self::HEADER_SIZE)
    }

    ///
    /// Sets the `next free block offset` field of the current free block at 
    /// `cur_free_offset` to `next`.
    /// 
    fn set_free_next(&mut self, cur_free_offset: usize, next: usize) {
        self.write_usize(cur_free_offset + Self::HEADER_SIZE, next);
    }

    fn resize_heap(&mut self, new_size: isize) -> Option<ProcCtlResizeHeapArgs> {
        let mut syscall_args = ProcCtlResizeHeapArgs {
            heap_base: 0,
            heap_size: 0,
            delta: new_size
        };
        let mut retval: usize = 0;

        syscall(Syscall::ProcControl {
            opcode: ProcCtlOpCode::ResizeHeap as usize,
            args: &mut syscall_args as *mut ProcCtlResizeHeapArgs as usize,
            ret_code: &mut retval as *mut usize as usize
        });

        if retval != size_of::<ProcCtlResizeHeapArgs>() {
            panic!("Bug in Syscall::ProcessControl/ResizeHeap");
        }
        if syscall_args.heap_base == 0 || syscall_args.heap_size == 0 {
            None
        } else {
            Some(syscall_args)
        }
    }
}


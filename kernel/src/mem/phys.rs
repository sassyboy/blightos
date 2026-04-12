//
// BlightOS Kernel
//
// Physical Memory Manager
//   Maintains a bitmap to track the status of each physical frame and provides
//   methods to allocate and free physical frames.
//
// 
#![allow(dead_code)]

use crate::util::*;

#[cfg(feature="debug_pmm")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[PMM] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_pmm"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}

//---------------------------------------------------------------------------//
// Public Data Types and Globals                                             //
//---------------------------------------------------------------------------//
#[derive(Copy, Clone, Debug)]
pub struct PMMapElement {
    pub base: usize,
    pub len:  usize,
    pub avail:bool,
}
impl PMMapElement {
    pub const fn new() -> Self {
        Self {
            base:   0,
            len:    0,
            avail:  false
        }
    }
}

//---------------------------------------------------------------------------//
// Private Data Types and Globals                                            //
//---------------------------------------------------------------------------//
pub const PHY_FRAME_SIZE: usize = 0x1000;
const PHY_FRAME_SHIFT: usize = 12;
struct FramesBitmap {
    total_frames:   usize, // Total installed RAM (in PHY_FRAME_SIZE)
    free_frames:    usize, // Free usable RAM (in PHY_FRAME_SIZE)
    first_phys_addr:usize, // 1st allocatable addr after kernel's image
    last_phys_addr: usize, // Last allocatable addr
    base:           usize, // Virt. addr. of bitmap in kernel's addr. space
    size:           usize, // Size of the bitmap in bytes
    // Red Zone: From the base address of the frame where the kernel is loaded
    //           to the end address of the frame where this bitmap is located
    //           cannot be allocated or freed.
    // To-do: Need a list of red zones for all the reserved/bad memory regions
    //        so that they are not freed by mistake and then allocated....
    red_zone_start: usize, // Kernel load address (not alloc/free-able)
    red_zone_end:   usize, // Last address of the bitmap (rounded to frame)
}
impl FramesBitmap {
    const fn new() -> Self {
        Self {
            total_frames:       0,
            free_frames:        0,
            first_phys_addr:    0,
            last_phys_addr:     0,
            base:               0,
            size:               0,
            red_zone_end:       0,
            red_zone_start:     0
        }
    }

    /// Returns true if the address is allocatable/freeable, i.e. it's within
    /// the physical memory range and not in the red zone
    fn is_valid(&self, addr: usize) -> bool {
        if addr < self.first_phys_addr || addr > self.last_phys_addr || 
            (addr >= self.red_zone_start && addr <= self.red_zone_end) {
            return false;
        }
        true
    }

    /// Returns true if the frame at the given address is marked as used (0)
    fn is_used(&self, addr: usize) -> bool {
        let index  = addr / PHY_FRAME_SIZE;
        let word_i = index / (size_of::<usize>() * 8);
        let bit_i  = index % (size_of::<usize>() * 8);
        let ptr    = (self.base as *mut usize).wrapping_add(word_i);
        unsafe { *ptr & (1 << bit_i) == 0 }
    }

    /// Marks the frame at the given address as used (0) or free (1) and updates
    /// the free frame count accordingly.
    fn mark(&mut self, addr: usize, used: bool) {
        let index  = addr / PHY_FRAME_SIZE;
        let word_i = index / (size_of::<usize>() * 8);
        let bit_i  = index % (size_of::<usize>() * 8);
        let ptr    = (self.base as *mut usize).wrapping_add(word_i);
        match used {
            true  => unsafe {
                // Mark USED if it's free
                if *ptr & (1 << bit_i) > 0 {
                    *ptr &= !(1 << bit_i);
                    self.free_frames -= 1;
                    // dbg!("Marked {:X} USED - word: {}, bit: {} \
                    //     bitmap_word: {:X}\n", addr, word_i, bit_i, *ptr);
                }
            },
            false => unsafe {
                // Mark FREE if it's used
                if *ptr & (1 << bit_i) == 0 {
                    *ptr |= 1 << bit_i;
                    self.free_frames += 1;
                    // dbg!("Marked {:X} FREE - word: {}, bit: {} \
                    //     bitmap_word: {:X}\n", addr, word_i, bit_i, *ptr);
                }
            }
        }
    }

    /// Marks a continuous range of frames as used or free. The range is defined by
    /// the start and end addresses (inclusive) and will be aligned to frame
    /// boundaries.
    fn mark_continuous(&mut self, start_addr: usize, end_addr: usize,
                                                                used: bool) {
        // Todo - more efficient to implement this and have mark_noblock call it
        let mut addr = start_addr;
        while addr < end_addr {
            self.mark(addr, used);
            addr += PHY_FRAME_SIZE;
        }
    }

}

static BITMAP : Spinlock<FramesBitmap> = Spinlock::new(FramesBitmap::new());


//---------------------------------------------------------------------------//
// Public Interface                                                          //
//---------------------------------------------------------------------------//
pub struct PhysMem {}
impl PhysMem {
    /// Initializes the physical memory manager by parsing the provided memory
    /// map, setting up the bitmap to track physical frames, and marking the
    /// kernel's own memory region as used to prevent accidental allocation.
    pub fn init(mmap: &[PMMapElement], kernel_start: usize, kernel_end: usize,
                                                kmod: Option<(usize, usize)>) {
        let mut last_alloc_ram : usize = 0;

        // 1) Find the last allocatable usable memory address and derive the
        //     size of the bitmap to represent that many frames
        for entry in mmap {
            dbg!("{:016X} - {:016X}: {}\n",
                    entry.base, entry.base + entry.len - 1,
                    match entry.avail {
                        true => "[USABLE]",
                        false=> "[RESERV]"
                    });
            match entry.avail {
                true    => {
                    if entry.base + entry.len > last_alloc_ram {
                        last_alloc_ram = entry.base + entry.len;
                    }
                }
                _       => {}
            }
        }
        let bitmap = &mut *(BITMAP.lock());
        bitmap.last_phys_addr= last_alloc_ram - 1;
        bitmap.total_frames  = last_alloc_ram / PHY_FRAME_SIZE;
        bitmap.size          = round_up!(bitmap.total_frames, 8);

        // 2) Allocate the bitmap (base) right after the kernel load addr, map 
        //    it to kernel's virtual address space (TODO), and mark it all as
        //    used (0). There may be unspecified memory regions in the map, and
        //    it's safer to assume they are unusable
        //    TODO: Make sure there is enough available memory for the bitmap
        bitmap.base = round_up!(kernel_end + 1, PHY_FRAME_SIZE); 
        if let Some((_, mod_end)) = kmod {
            // There is a module loaded after the kernel. Place the bitmap after
            // that so that it doesn't overwrite the loaded module
            if mod_end > kernel_end {
                bitmap.base = round_up!(mod_end + 1, PHY_FRAME_SIZE);
            }
        }
    
        unsafe {
            (bitmap.base as *mut u8).write_bytes(0, bitmap.size);
        }
        bitmap.free_frames = 0;
    
        // 3) Mark any usable memory as free
        dbg!("Total Frames: {}\n", bitmap.total_frames);
        dbg!("Free Frames before acouting for holes: {}\n", bitmap.free_frames);
        for entry in mmap {
            match entry.avail {
                true    => {
                    let start = round_up!(entry.base, PHY_FRAME_SIZE);
                    let end = round_down!(start + entry.len, PHY_FRAME_SIZE) -1;
                    if start < bitmap.last_phys_addr {
                        bitmap.mark_continuous(start, end, false);
                    }
                },
                false   => {}
            }
        }

        // 4) Mark any unavailable memory that overlaps the free memory as used
        for entry in mmap {
            match entry.avail {
                false => {
                    let start = round_up!(entry.base, PHY_FRAME_SIZE);
                    let end = round_down!(start + entry.len, PHY_FRAME_SIZE) -1;
                    if start < bitmap.last_phys_addr {
                        bitmap.mark_continuous(start, end, true);
                    } 
                },
                true => {}
            }
        }

        // 5) Mark the kernel load address and bitmap itself as used and redzone
        //    it.
        bitmap.red_zone_start= round_down!(kernel_start, PHY_FRAME_SIZE);
        bitmap.red_zone_end  = bitmap.base + bitmap.size;
        bitmap.red_zone_end  = round_up!(bitmap.red_zone_end, PHY_FRAME_SIZE)-1;
        bitmap.mark_continuous(bitmap.red_zone_start, bitmap.red_zone_end, true);
        dbg!("Kernel loaded from {:X} to {:X} ({:.2} KBs)\n",
                kernel_start, kernel_end,
                (kernel_end - kernel_start) as f64 /1024.0);
        if let Some((mod_start, mod_end)) = kmod {
            bitmap.mark_continuous(mod_start, mod_end, true);
            dbg!("KMod marked used from {:X} to {:X}\n", mod_start, mod_end);
        }
        dbg!("PMM Bitmap from {:X} to {:X} (maps {} frames) - ",
            bitmap.base, bitmap.base + bitmap.size - 1, bitmap.total_frames);
        dbg!("Red Zone from {:X} to {:X} ({:.2} KBs)\n",
            bitmap.red_zone_start, bitmap.red_zone_end,
            (bitmap.red_zone_end - bitmap.red_zone_start) as f64 / 1024.0);
        klog!("Free Frames: {} ({:.2} MBs)\n", bitmap.free_frames,
            (bitmap.free_frames << 12) as f64 / (1024.0*1024.0));
    }

    //
    // Static physical memory frame management methods
    //

    /// Checks if the frame at the given physical address is currently allocated
    /// (true) or free (false).
    /// Out-of-bound addresses are considered allocated
    pub fn is_used(addr: usize) -> bool {
        let bitmap = &mut *(BITMAP.lock());
        if bitmap.is_valid(addr) {
            return bitmap.is_used(addr);
        }
        true // Out-of-bound addrs - Pretend it's already used up
    }

    /// Marks the frame at the given physical address as allocated (used=true)
    /// or free (used=false). Out-of-bound addresses are ignored.
    pub fn mark(addr: usize, used: bool) {
        let bitmap = &mut *(BITMAP.lock());
        if bitmap.is_valid(addr) {
            bitmap.mark(addr, used);
        }
    }

    /// Marks a continuous range of frames as allocated or free.
    pub fn mark_continuous(addr: usize, num_frames: usize, used: bool) {
        let bitmap = &mut *(BITMAP.lock());
        for i in 0..num_frames {
            if bitmap.is_valid(addr) {
                bitmap.mark(addr + (i * PHY_FRAME_SIZE), used);
            }
        }
    }

    /// Returns the total number of physical frames in the system
    pub fn total_frame_count() -> usize {
        let bitmap = &mut *(BITMAP.lock());
        bitmap.total_frames
    }

    /// Returns the total size of physical memory in the system in bytes
    pub fn total_memory() -> usize {
        Self::total_frame_count() * PHY_FRAME_SIZE
    }

    /// Returns the total number of free physical frames in the system
    pub fn free_frame_count() -> usize {
        let res: usize;
        {
            let bitmap = &mut *(BITMAP.lock());
            res = bitmap.free_frames;
        }
        res
    }

    /// Returns the total size of free physical memory in the system in bytes
    pub fn free_memory() -> usize {
        Self::free_frame_count() * PHY_FRAME_SIZE
    }

    //
    // Dynamic physical memory frame allocator
    // First-fit - Inefficient, but will do for now
    //

    /// Allocates multiple physical frames and returns their addresses in the
    /// provided slice. The number of frames allocated is returned as well.
    /// The frames are not guaranteed to be continuous.
    /// Throws:
    ///     - OutOfMemory error if the requested number of free frames are not
    ///       available. In this case, no frames will be allocated and any
    ///       partial allocations will be rolled back.
    pub fn alloc_frames(frames: &mut [usize]) -> Result<usize, Error> {
        let mut allocated = 0;
        let bitmap = &mut *(BITMAP.lock());
        if bitmap.free_frames < frames.len() {
            // Not enough free frames available
            return Err(error!(ErrorCode::OutOfMemory));
        }
        let mut addr = bitmap.first_phys_addr;
        while addr < bitmap.last_phys_addr {
            if bitmap.is_used(addr) == false {
                bitmap.mark(addr, true);
                frames[allocated] = addr;
                allocated += 1;
                if allocated == frames.len() {
                    break;
                }
            }
            addr += PHY_FRAME_SIZE;
        }
        if allocated == frames.len() {
            Ok(allocated)
        } else {
            // Not enough free frames available. Roll back any partial
            // allocations. This shouldn't happen because we check the free
            // frame count at the beginning, but any synchronization issues or
            // bugs in the bitmap management could lead to this.
            klog!("Bug: alloc_frames requested {} frames, allocated {}. \
                    Rolling back.\n", frames.len(), allocated);
            for i in 0..allocated {
                bitmap.mark(frames[i], false);
            }
            Err(error!(ErrorCode::OutOfMemory))
        }
    }

    /// Allocates multiple physical frames starting from the end of the physical
    /// memory and returns their addresses in the provided slice. The number of
    /// frames allocated is returned as well. The frames are not guaranteed to be
    /// continuous.
    /// Throws:
    ///     - OutOfMemory error if the requested number of free frames are not
    ///       available. In this case, no frames will be allocated and any
    ///       partial allocations will be rolled back.
    pub fn alloc_high_frames(frames: &mut [usize]) -> Result<usize, Error> {
        let mut allocated = 0;
        {
            let bitmap = &mut *(BITMAP.lock());
            if bitmap.free_frames < frames.len() {
                // Not enough free frames available
                return Err(error!(ErrorCode::OutOfMemory));
            }
            // Start from the last physical frame and move backwards to find
            // free frames.
            let mut addr = bitmap.last_phys_addr & !(PHY_FRAME_SIZE - 1);
            while addr >= bitmap.first_phys_addr{
                if bitmap.is_used(addr) == false {
                    bitmap.mark(addr, true);
                    frames[allocated] = addr;
                    allocated += 1;
                    if allocated == frames.len() {
                        break;
                    }
                }
                addr -= PHY_FRAME_SIZE;
            }
        }
        if allocated == frames.len() {
            frames.reverse(); // Reverse to maintain ascending order of addresses
            Ok(allocated)
        } else {
            // Not enough free frames available. Roll back any partial
            // allocations. This shouldn't happen because we check the free
            // frame count at the beginning, but any synchronization issues or
            // bugs in the bitmap management could lead to this.
            klog!("Bug: alloc_frames_reverse requested {} frames, allocated {}. \
                    Rolling back.\n", frames.len(), allocated);
            {
                let bitmap = &mut *(BITMAP.lock());
                for i in 0..allocated {
                    bitmap.mark(frames[i], false);
                }
            }
            Err(error!(ErrorCode::OutOfMemory))
        }
    }

    /// Frees multiple physical frames given their addresses in the provided
    /// slice.
    pub fn free_frames(frames: &[usize]) {
        let bitmap = &mut *(BITMAP.lock());
        for &addr in frames {
            if bitmap.is_valid(addr) && bitmap.is_used(addr) {
                bitmap.mark(addr, false);
            }
        }
    }

    /// Allocates a single physical frame and returns its address.
    /// Throws:
    ///     - OutOfMemory error if no free frames are available.
    pub fn alloc() -> Result<usize, Error> {
        let mut addr: [usize; 1] = [0];
        let cnt = Self::alloc_frames(&mut addr)?;
        if cnt == 1 {
            Ok(addr[0])
        } else {
            // This shouldn't happen because alloc_frames should either return
            // 1 or an error, but just in case, we handle the unexpected case
            klog!("Bug: alloc_frame allocated {} frames instead of 1. \
                    Rolling back.\n", cnt);
            Self::free_frames(&addr[0..cnt]);
            Err(error!(ErrorCode::OutOfMemory))
        }
    }

    /// Frees a single physical frame given its address.
    pub fn free(addr: usize) {
        let bitmap = &mut *(BITMAP.lock());
        if bitmap.is_valid(addr) && bitmap.is_used(addr) {
            bitmap.mark(addr, false);
        }
    }

    /// Allocates `num_frames` number of physical frames and returns the 
    /// address starting. The frames allocated by this method are guaranteed to
    /// be continuous in physical memory.
    /// Throws:
    ///     - OutOfMemory error `num_frames` continuous free frames are not
    ///       available.
    pub fn alloc_continuous(num_frames: usize) -> Result<usize, Error> {
        let _free_before:   usize;
        let _free_after:    usize;
        let mut result:     Option<usize> = None;
        {
            let bitmap = &mut *(BITMAP.lock());
            if num_frames > bitmap.free_frames {
                // Not enough free frames available
                klog!("alloc_continuous({}) Failed - Not enough free frames. \
                        Free Frames: {}\n", num_frames, bitmap.free_frames);
                return Err(error!(ErrorCode::OutOfMemory));
            }
            let mut addr = bitmap.first_phys_addr;
            let mut ret = addr;
            let mut cnt = 0;

            _free_before = bitmap.free_frames;
            while addr < bitmap.last_phys_addr {
                if bitmap.is_used(addr) == false {
                    cnt += 1;
                    if cnt == 1 {
                        ret = addr;
                    }
                    if cnt == num_frames {
                        bitmap.mark_continuous(ret, addr + PHY_FRAME_SIZE -1,
                                                                        true);
                        result = Some(ret);
                        break;
                    }
                } else {
                    ret = 0;
                    cnt = 0;
                }
                addr += PHY_FRAME_SIZE;
            }
            _free_after = bitmap.free_frames;
        } // BITMAP.unlock
        match result {
            Some(adr)   => {
                dbg!("palloc_continuous({}) Granted @ {:X}, \
                        FreeFrames: {} -> {}\n", num_frames, adr, _free_before,
                                                                _free_after);
                return Ok(adr);
            },
            None         => {
                dbg!("palloc_continuous({}) Failed, FreeFrames: {} -> {}\n",
                        num_frames, _free_before, _free_after);
                return Err(error!(ErrorCode::OutOfMemory));
            }
        }
    }

    pub fn free_continuous(addr: usize, num_frames: usize) {
        let _free : usize;
        {
            let bitmap = &mut *(BITMAP.lock());
            for i in 0..num_frames {
                let base = addr + i * PHY_FRAME_SIZE;
                if bitmap.is_valid(base) && bitmap.is_used(base) {
                    bitmap.mark(base, false);
                }
            }
            _free = bitmap.free_frames;
        } // BITMAP.unlock
        dbg!("pfree_continuous({:X}, {}) - Free Frames: {}\n",
                addr, num_frames, _free);
    }
}

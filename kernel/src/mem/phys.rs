//
// BlightOS Kernel
//
// Physical Memory Manager
//   Marks physical memory frames as free (1) or allocated (0)
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
    pub total_frames:   usize, // Total installed RAM (in PHY_FRAME_SIZE)
    pub free_frames:    usize, // Free usable RAM (in PHY_FRAME_SIZE)
    pub first_phys_addr:usize, // 1st allocatable addr after kernel's image
    pub last_phys_addr: usize, // Last allocatable addr
    pub base:           usize, // Virt. addr. of bitmap in kernel's addr. space
    pub size:           usize, // Size of the bitmap in bytes
    // Red Zone: From the base address of the frame where the kernel is loaded
    //           to the end address of the frame where this bitmap is located
    //           cannot be allocated or freed.
    // To-do: Need a list of red zones for all the reserved/bad memory regions
    //        so that they are not freed by mistake and then allocated....
    pub red_zone_start: usize, // Kernel load address (not alloc/free-able)
    pub red_zone_end:   usize, // Last address of the bitmap (rounded to frame)
}
impl FramesBitmap {
    pub const fn new() -> Self {
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
}

static BITMAP : Spinlock<FramesBitmap> = Spinlock::new(FramesBitmap::new());


//---------------------------------------------------------------------------//
// Public Interface                                                          //
//---------------------------------------------------------------------------//

pub fn pmm_init(mmap: &[PMMapElement], kernel_start: usize, kernel_end: usize,
                kmod: Option<(usize, usize)>) {
    let mut last_alloc_ram : usize = 0;

    // 1) Find the last allocatable usable memory address (and print the map)
    //    and derive the size of the bitmap to represent that many frames
    for entry in mmap {
        dbg!("{:016X} - {:016X}: {}\n",
                entry.base, entry.base + entry.len - 1,
                match entry.avail {
                    true => "[USABLE]",
                    false=> "[RESERV]"
                }
        );
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

    // 2) Allocate the bitmap (base) right after the kernel load addr, map it
    //    to kernel's virtual address space (TODO), and mark it all as used (0).
    //    There may be unspecified memory regions in the map, and it's safer to
    //    assume they are unusable
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
                    pmm_mark_continuous_nolock(bitmap, start, end, false);
                }
            },
            false   => {}
        }
    }

    // 4) Mark any unavailable memory that overlaps the free memory as used
    for entry in mmap {
        match entry.avail {
            false    => {
                let start = round_up!(entry.base, PHY_FRAME_SIZE);
                let end = round_down!(start + entry.len, PHY_FRAME_SIZE) -1;
                if start < bitmap.last_phys_addr {
                    pmm_mark_continuous_nolock(bitmap, start, end, true);
                } 
            },
            true   => {}
        }
    }

    // 5) Mark the kernel load address and bitmap itself as used and redzone it
    bitmap.red_zone_start= round_down!(kernel_start, PHY_FRAME_SIZE);
    bitmap.red_zone_end  = bitmap.base + bitmap.size;
    bitmap.red_zone_end  = round_up!(bitmap.red_zone_end, PHY_FRAME_SIZE) - 1;
    pmm_mark_continuous_nolock(bitmap,
                            bitmap.red_zone_start, bitmap.red_zone_end, true);
    dbg!("Kernel loaded from {:X} to {:X} ({:.2} KBs)\n",
        kernel_start, kernel_end,
        (kernel_end - kernel_start) as f64 /1024.0);
    if let Some((mod_start, mod_end)) = kmod {
        pmm_mark_continuous_nolock(bitmap, mod_start,mod_end, true);
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

// Returns true if the address is allocatable/freeable
fn pmm_valid_address_nolock(bitmap: &FramesBitmap, addr: usize) -> bool {
    if addr < bitmap.first_phys_addr || addr > bitmap.last_phys_addr || 
      (addr >= bitmap.red_zone_start && addr <= bitmap.red_zone_end) {
        return false;
    }
    true
}

fn pmm_is_used_nolock(bitmap: &FramesBitmap, addr: usize) -> bool {
    let index  = addr / PHY_FRAME_SIZE;
    let word_i = index / (size_of::<usize>() * 8);
    let bit_i  = index % (size_of::<usize>() * 8);
    let ptr    = (bitmap.base as *mut usize).wrapping_add(word_i);
    unsafe {
        *ptr & (1 << bit_i) == 0
    }
}

fn pmm_mark_nolock(bitmap: &mut FramesBitmap, addr: usize, used: bool){
    let index  = addr / PHY_FRAME_SIZE;
    let word_i = index / (size_of::<usize>() * 8);
    let bit_i  = index % (size_of::<usize>() * 8);
    let ptr    = (bitmap.base as *mut usize).wrapping_add(word_i);
    match used {
        true  => unsafe {
            // Mark USED if it's free
            if *ptr & (1 << bit_i) > 0 {
                *ptr &= !(1 << bit_i);
                bitmap.free_frames -= 1;
                // dbg!("Marked {:X} USED - word: {}, bit: {} bitmap_word: {:X}\n",
                //     addr, word_i, bit_i, *ptr);
            }
        }
        false => unsafe {
            // Mark FREE if it's used
            if *ptr & (1 << bit_i) == 0 {
                *ptr |= 1 << bit_i;
                bitmap.free_frames += 1;
                // dbg!("Marked {:X} FREE - word: {}, bit: {} bitmap_word: {:X}\n",
                //     addr, word_i, bit_i, *ptr);
            }
        }
    }
}


fn pmm_mark_continuous_nolock(bitmap: &mut FramesBitmap,
                              start_addr: usize, end_addr: usize, used: bool) {
    // Todo - more efficient to implement this and have mark_noblock call this
    let mut addr = start_addr;
    while addr < end_addr {
      pmm_mark_nolock(bitmap, addr, used);
      addr += PHY_FRAME_SIZE;
    }
}

pub fn pmm_is_used(addr: usize) -> bool {
    let bitmap = &mut *(BITMAP.lock());
    if pmm_valid_address_nolock(bitmap, addr) {
        return pmm_is_used_nolock(bitmap, addr);
    }
    true // Out-of-bound addrs - Pretend it's already used up
}

pub fn pmm_mark(addr: usize, used: bool) {
    let bitmap = &mut *(BITMAP.lock());
    if pmm_valid_address_nolock(bitmap, addr) {
        pmm_mark_nolock(bitmap, addr, used);
    }
}

pub fn pmm_mark_continuous(addr: usize, num_frames: usize, used: bool) {
    let bitmap = &mut *(BITMAP.lock());
    for i in 0..num_frames {
        if pmm_valid_address_nolock(bitmap, addr) {
            pmm_mark_nolock(bitmap, addr + (i * PHY_FRAME_SIZE), used);
        }
    }
}

pub fn pmm_num_total_frames() -> usize {
    let bitmap = &mut *(BITMAP.lock());
    bitmap.total_frames
}

pub fn pmm_num_free_frames() -> usize {
    let res: usize;
    {
        let bitmap = &mut *(BITMAP.lock());
        res = bitmap.free_frames;
    }
    res
}

//
// Dynamic physical memory frame allocator
// First-fit - Highly inefficient, but will do for now
//
pub fn palloc() -> Option<usize> {
    let _free_before:       usize;
    let _free_after:        usize;
    let mut result:         Option<usize> = None;
    {
        let bitmap = &mut *(BITMAP.lock());
        let mut addr : usize = bitmap.first_phys_addr;
    
        _free_before = bitmap.free_frames;
        while addr < bitmap.last_phys_addr {
            if pmm_is_used_nolock(bitmap, addr) == false {
                pmm_mark_nolock(bitmap, addr, true);
            
                result = Some(addr);
                break;
            }
            addr += PHY_FRAME_SIZE;
        }
        _free_after = bitmap.free_frames;
    }  // BITMAP.unlock
    match result {
        Some(_adr)   => {
            dbg!("palloc() Granted @ {:X}, FreeFrames: {} -> {}\n",
                _adr, _free_before, _free_after);
        }
        None         => {
            dbg!("palloc() Failed, FreeFrames: {} -> {}\n",
                _free_before, _free_after);
        }
    }
    result
}

pub fn palloc_continuous(num_frames: usize) -> Option<usize> {
    let _free_before:   usize;
    let _free_after:    usize;
    let mut result:     Option<usize> = None;
    {
        let bitmap = &mut *(BITMAP.lock());
        let mut addr = bitmap.first_phys_addr;
        let mut ret = addr;
        let mut cnt = 0;

        _free_before = bitmap.free_frames;
        while addr < bitmap.last_phys_addr {
            if pmm_is_used_nolock(bitmap, addr) == false {
                cnt += 1;
                if cnt == 1 {
                    ret = addr;
                }
                if cnt == num_frames {
                    pmm_mark_continuous_nolock(bitmap,
                                          ret, addr + PHY_FRAME_SIZE -1, true);
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
        Some(_adr)   => {
            dbg!("palloc_continuous({}) Granted @ {:X}, FreeFrames: {} -> {}\n",
                num_frames, _adr, _free_before, _free_after);
        }
        None         => {
            dbg!("palloc_continuous({}) Failed, FreeFrames: {} -> {}\n",
                num_frames, _free_before, _free_after);
        }
    }
    result
}

pub fn pfree_continuous(addr: usize, num_frames: usize) {
    let _free : usize;
    {
        let bitmap = &mut *(BITMAP.lock());
        for i in 0..num_frames {
            let base = addr + i * PHY_FRAME_SIZE;
            if pmm_valid_address_nolock(bitmap, base) {
                if pmm_is_used_nolock(bitmap, base) == true {
                    pmm_mark_nolock(bitmap, base, false);
                }
            }
        }
        _free = bitmap.free_frames;
    } // BITMAP.unlock
    dbg!("pfree_continuous({:X}, {}) - Free Frames: {}\n", addr, num_frames,
            _free);
}

pub fn pfree(addr: usize) {
    let _free : usize;
    {
        let bitmap = &mut *(BITMAP.lock());
        if pmm_valid_address_nolock(bitmap, addr) {
            if pmm_is_used_nolock(bitmap, addr) == true {
                pmm_mark_nolock(bitmap, addr, false);
            }
        }
        _free = bitmap.free_frames;
    } // BITMAP.unlock
    dbg!("pfree({:X}) - Free Frames: {}\n", addr, _free);
}


// TODO: A UNIT TEST ROTUINE HERE!
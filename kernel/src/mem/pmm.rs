//
// BlightOS Kernel
//
// Physical Memory Manager
//   Marks physical memory frames as free or allocated
//
// 
#![allow(dead_code)]

use crate::util::*;

//---------------------------------------------------------------------------//
// Public Data Types and Globals                                             //
//---------------------------------------------------------------------------//
#[derive(Copy, Clone)]
pub struct PMMapElement {
    pub base: usize,
    pub len:  usize,
    pub avail:bool,
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

pub fn pmm_init(mmap: &[PMMapElement], kernel_start: usize, kernel_end: usize) {
    let mut last_alloc_ram : usize = 0;

    // 1) Find the last allocatable usable memory address (and print the map)
    //    and derive the size of the bitmap to represent that many frames
    for entry in mmap {
        klog!("{:016X} - {:016X}: {}\n",
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
    //    to kernel's virtual address space (TODO), and mark it all as used.
    //    There may be unspecified memory regions in the map, and it's safer to
    //    assume they are unusable
    //    TODO: Make sure there is enough available memory for the bitmap
    bitmap.base = round_up!(kernel_end + 1, PHY_FRAME_SIZE);
    unsafe {
        raw_memset(bitmap.base, bitmap.size, 0xFF);
    }
    bitmap.free_frames = 0;
    
    // 3) Mark any usable memory as free
    // klog!("Total Frames: {}\n", bitmap.total_frames);
    // klog!("Free Frames before acouting for holes: {}\n", bitmap.free_frames);
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

    // 4) Mark the kernel load address and bitmap itself as used and redzone it
    bitmap.red_zone_start= round_down!(kernel_start, PHY_FRAME_SIZE);
    bitmap.red_zone_end  = bitmap.base + bitmap.size;
    bitmap.red_zone_end  = round_up!(bitmap.red_zone_end, PHY_FRAME_SIZE) - 1;
    pmm_mark_continuous_nolock(bitmap,
                            bitmap.red_zone_start, bitmap.red_zone_end, true);
    
    klog!("Kernel loaded from {:X} to {:X}\n", kernel_start, kernel_end);
    klog!("PMM Bitmap from {:X} to {:X} (maps {} frames) - ",
        bitmap.base, bitmap.base + bitmap.size - 1, bitmap.total_frames);
    klog!("Red Zone from {:X} to {:X}\n", bitmap.red_zone_start, bitmap.red_zone_end);
    klog!("Free Frames: {}\n", bitmap.free_frames);
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
    let index = addr / PHY_FRAME_SIZE;
    let ptr = (bitmap.base + index/8) as *mut u8;
    unsafe {
        *ptr & (1 << (index % 8) as u8) > 0
    }
}

fn pmm_mark_nolock(bitmap: &mut FramesBitmap, addr: usize, used: bool){
    let index = addr / PHY_FRAME_SIZE;
    let ptr = (bitmap.base + index/8) as *mut u8;
    match used {
        true  => unsafe { // Mark used if it's not already used
            if *ptr & (1 << (index % 8) as u8) == 0 {
                *ptr |= 1 << (index % 8) as u8;
                bitmap.free_frames -= 1;
            }
        }
        false => unsafe { // Mark free if it's not already free
            if *ptr & (1 << (index % 8) as u8) > 0 {
                *ptr &= !(1 << (index % 8));
                bitmap.free_frames += 1;
            }
        }
    }
}

fn pmm_mark_continuous_nolock(bitmap: &mut FramesBitmap,
                              start_addr: usize, end_addr: usize, used: bool) {
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

pub fn pmm_num_total_frames() -> usize {
    let bitmap = &mut *(BITMAP.lock());
    bitmap.total_frames
}

pub fn pmm_num_free_frames() -> usize {
    let bitmap = &mut *(BITMAP.lock());
    bitmap.free_frames
}

//
// Dynamic physical memory frame allocator
// First-fit - Highly inefficient, but will do for now
//
pub fn palloc() -> Option<usize> {
    let bitmap = &mut *(BITMAP.lock());
    let mut addr : usize = bitmap.first_phys_addr;
    while addr < bitmap.last_phys_addr {
        if pmm_is_used_nolock(bitmap, addr) == false {
            pmm_mark_nolock(bitmap, addr, true);
            return Some(addr);
        }
        addr += PHY_FRAME_SIZE;
    }
    None
}

pub fn palloc_continuous(num_frames: usize) -> Option<usize> {
    let bitmap = &mut *(BITMAP.lock());
    let mut addr = bitmap.first_phys_addr;
    let mut ret = addr;
    let mut cnt = 0;
    
    while addr < bitmap.last_phys_addr {
        if pmm_is_used_nolock(bitmap, addr) == false {
            cnt += 1;
            if cnt == 1 {
              ret = addr;
            }
            if cnt == num_frames {
                pmm_mark_continuous_nolock(bitmap,
                                          ret, addr + PHY_FRAME_SIZE -1, true);
                return Some(ret);
            }
        } else {
          ret = 0;
          cnt = 0;
        }
        addr += PHY_FRAME_SIZE;
    }
    None
}

pub fn pfree(addr: usize){
    let bitmap = &mut *(BITMAP.lock());
    if pmm_valid_address_nolock(bitmap, addr) {
        if pmm_is_used_nolock(bitmap, addr) == true {
            pmm_mark_nolock(bitmap, addr, false);
        }
    }
}


// TODO: A UNIT TEST ROTUINE HERE!
//
// Graphical User Interface Server/Driver
//
// Provides a VFS mount-point for mediated Window management
// This is unlike the direct hardware access the kbd:/ or framebuffer:/
// mount points provide.
//
// The VFS interface:
// gui:/        : No READ/WRITE/ENUM/EXEC allowed on the root for now.
//  |-- cursor  : TODO: to get/set mouse pointer's properties
//  |-- window  : - An open on this file creates a new window with a graphical
//                  context mapped to both kmap and the dmap of the calling
//                  process. The window object is then pushed to the back of the
//                  list (i.e., highest z-order = in front of everything else)
//                - A write to this file adds a message for the window that can
//                  be read by the window's user-space render logic, etc.
//                - A read from this file returns the current outstanding
//                  messages for this window.
//                - Other actions, e.g., resize, move, minimize, etc. are
//                  performed via an Exec call.
//                - Closing the file will destroy the window.
//
// NOTE: The first window must be opened by the INIT program, which is treated
//       as the desktop. i.e., must be full-screen, cannot be brought to front
//       (z-index=0) unless if every other window is minimized.

use core::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use core::slice::{from_raw_parts, from_raw_parts_mut};
use alloc::string::String;
use alloc::vec::Vec;
use crate::arch::{MMUMapping, MMUTrait};
use crate::mem::MemoryType;
use crate::mem::virt::AddressSpace;
use crate::{PHY_FRAME_SIZE, PhysMem, copy_from_user, copy_to_user};
use crate::drivers::video::framebuffer::FrameBuffer;
use crate::drivers::input::Mouse;
use crate::fs::{DirectoryEntry, FileOperation, MountPoint};
use crate::util::*;

#[cfg(feature="debug_gui")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[GUI] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_gui"))]
macro_rules! dbg{
    ($($arg:tt)*) => { };
}

struct Window {
    hnd:        usize,
    phys_base:  usize,  // Base physical address of the graphical context, i.e.,
                        // shadow framebuffer containing the window's graphics.
    num_frames: u32,    // # of physical frames allocated for the context.
    dmap_base:  usize,  // Base virtual address the user-space uses to 
                        // update the graphical context of the window
    kmap_base:  usize,  // Base virtual address the kernel-space render
                        // task accesses the window's context.
    flags:      u64,
    left:       u32,
    top:        u32,
    width:      u32,
    height:     u32,
    redraw:     bool,   // Either this window, or some other window overlapping
                        // this one has changed, and this window should be
                        // redrawn
}
impl Window {
    // FLG_DESKTOP: The window cannot be minimized or hid or brought to front
    // pub const FLG_DESKTOP:          u64 = 0x1;
    pub const FLG_HIDDEN:           u64 = 0x2;
    // pub const FLG_MOVEABLE:         u64 = 0x3;
    // pub const FLG_RESIZABLE:        u64 = 0x4;

    pub const fn new(hnd: usize) -> Self {
        Self {
            hnd,
            phys_base:  0,
            num_frames: 0,
            dmap_base:  0,
            kmap_base:  0,
            flags:      Self::FLG_HIDDEN,
            left:       0,
            top:        0,
            width:      0,
            height:     0,
            redraw:     true
        }
    }

    /// Allocates a graphical context for the window that's mapped to the user
    /// space and the kernel space (sharing the same physical frames)
    pub fn init(&mut self) -> Result<(), Error> {
        // Calculate the size of the context required
        let scr_size = FrameBuffer::screen_size(); // (height, width)
        let buf_size = scr_size.0 * scr_size.1 * 4; // RGBA
        let num_frames = div_round_up!(buf_size, PHY_FRAME_SIZE as u32);

        // Allocate the required #of frames in a contiguous fashion
        self.phys_base = PhysMem::alloc_continuous(num_frames as usize)?;
        self.num_frames= num_frames;

        // Map it to the kmap pool for our render task
        self.kmap_base = MMUMapping::kmap(self.phys_base, num_frames as usize,
                            MemoryType::Normal).expect("Out of kmap space!");

        // Map it to the dmap pool for the user-space
        let mut frames = Vec::<usize>::with_capacity(num_frames as usize);
        for i in 0..num_frames as usize {
            frames.push(self.phys_base + i * PHY_FRAME_SIZE);
        }
        self.dmap_base = AddressSpace::dmap(frames.as_slice())
                                        .expect("Out of dmap space!");
        Ok(())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.num_frames == 0 {
            return;
        }
        // Unmap the context from kmap
        MMUMapping::kunmap(self.kmap_base, self.num_frames as usize);

        // Unmap the context from dmap
        // BUG: At this point we are in a close system call and the process
        // struct is locked from kernel.rs, so, we can't lock it again here.
        // the kernel may need to send us the process structure
        // AddressSpace::dunmap(self.dmap_base, self.num_frames as usize)
        //                             .expect("BUG: dunmap in destroy)_window");

        // Free the physical frames as they are not cleaned up when the process
        // terminates.
        PhysMem::free_continuous(self.phys_base, self.num_frames as usize);
        dbg!("Window.drop({:X}): Freed {} frames from kmap:{:X} & phys:{:X}\n",
            self.hnd, self.num_frames, self.kmap_base, self.phys_base);
    }
}

static WIN_LIST: Spinlock<Vec<Window>> = Spinlock::new(Vec::new());
static NEXT_WHND: AtomicUsize = AtomicUsize::new(GUI::HND_WINDOW0);

pub struct GUI {
    cur_buf:    usize,
    pre_buf:    usize,
    sfb_vaddr:  [usize; 2],
    sc_height:  usize,
    sc_width:   usize,
    pxl_cnt:    usize,
}

static INSTANCE: Spinlock<GUI> = Spinlock::new(GUI::new());

impl GUI {
    pub const fn new() -> Self {
        Self {
            cur_buf:    0,
            pre_buf:    1,
            sfb_vaddr:  [0, 0],
            sc_height:  0,
            sc_width:   0,
            pxl_cnt:    0,
        }
    }

    pub fn enumerate() -> usize {
        let mnt_obj = MountPoint {
            name:       String::from("gui"),
            fops:       Self::fops_handler
        };
        if MountPoint::mount(mnt_obj) {
            return 1;
        }
        0
    }

    pub fn post_enum() {
        // Allocated and initialize our shadow buffers
        let mut gui = INSTANCE.lock();
        let scr_size = FrameBuffer::screen_size(); // (height, width)
        gui.sc_height = scr_size.0 as usize;
        gui.sc_width  = scr_size.1 as usize;
        gui.pxl_cnt   =   gui.sc_height * gui.sc_width;
        let buf_size  = gui.pxl_cnt * 3; // RGB
        let frm_cnt   = div_round_up!(buf_size, PHY_FRAME_SIZE);
        // Allocate and kmap the shadow framebuffers
        for i in 0..2 {
            let Ok(paddr) = PhysMem::alloc_continuous(frm_cnt) else {
                panic!("Not enough physical memory!");
            };
            let Some(vaddr) = MMUMapping::kmap(paddr, frm_cnt,
                                                    MemoryType::Normal) else {
                panic!("Out of kmap space!");
            };
            gui.sfb_vaddr[i] = vaddr;            
        }
        let sfb: [&mut[(u8,u8,u8)]; 2] = 
        unsafe {[
            from_raw_parts_mut(gui.sfb_vaddr[0] as *mut(u8,u8,u8), gui.pxl_cnt),
            from_raw_parts_mut(gui.sfb_vaddr[1] as *mut(u8,u8,u8), gui.pxl_cnt)
        ]};
        sfb[0].fill((0, 0, 0));
        sfb[1].fill((0, 0, 0));
        // Initialize Mouse and its cursor
        Mouse::reset_coordinates(gui.sc_width as u32 / 2,
                                gui.sc_height as u32 / 2,
                                gui.sc_width as u32,
                                gui.sc_height as u32);
        Mouse::set_irq_callback(Self::update_mouse);
    }

    pub fn release(_dev_id: usize) {
        // not needed 
    }

    pub const HND_ROOT:             usize = 0x0;
    pub const HND_WINDOW0:          usize = 0x10;
    pub const MAX_WINDOW_BUFFER:    usize = 4096 * 4096 * 4;

    // Sets the properties of a window (flags, position and size)
    // The input buffer is of type WindowProperties
    pub const WIN_FUNC_SET:         usize = 0x1;
    pub const WIN_FUNC_GET:         usize = 0x2;
    pub const WIN_FUNC_UPDATE:      usize = 0x3;

    fn fops_handler(op: FileOperation) -> Result<usize, Error> {
        match op {
            FileOperation::Open { full_path, mode: _, dent } => {
                let mpath = MountPoint::device_relative_path(full_path);
                if mpath.eq("/") {
                    dent.name = String::from("");
                    dent.size = 0;
                    dent.flags = DirectoryEntry::DEV_R_DIR_FLAGS;
                    return Ok(Self::HND_ROOT);
                } else if mpath.eq("/window") {
                    dent.name = String::from("window");
                    dent.size = Self::MAX_WINDOW_BUFFER;
                    dent.flags = DirectoryEntry::DEV_RWX_FILE_FLAGS;
                    let whnd = NEXT_WHND.fetch_add(1, SeqCst);
                    Self::create_window(whnd)?;
                    return Ok(whnd);
                } else {
                    return Err(error!(ErrorCode::InvalidPath));
                }
            },
            FileOperation::Enum { hnd: _, out: _ } => {
                // No need to show listings in user space
                return Err(error!(ErrorCode::InvalidOp));
            },
            FileOperation::Close { hnd } => {
                if hnd >= Self::HND_WINDOW0 && hnd <= NEXT_WHND.load(SeqCst) {
                    Self::destroy_window(hnd);
                }
                return Ok(0);
            },
            FileOperation::Exec {hnd, func, buff} => {
                if hnd >= Self::HND_WINDOW0 && hnd <= NEXT_WHND.load(SeqCst) {
                    if func == Self::WIN_FUNC_SET {
                        return Self::set_window(hnd, buff);
                    } else if func == Self::WIN_FUNC_GET {
                        return Self::get_window(hnd, buff);
                    } else if func == Self::WIN_FUNC_UPDATE {
                        return Self::update_window(hnd);
                    }
                    return Err(error!(ErrorCode::InvalidArgument));
                }
                return Err(error!(ErrorCode::InvalidHandle));
            },
            FileOperation::Read { hnd: _, off: _, buff: _ } => {
                return Err(error!(ErrorCode::InvalidHandle));
            }
            _ => {
                return Err(error!(ErrorCode::InvalidOp));
            }
        }
    }

    /// Creates a new window and adds it to the list. The window is initially
    /// flagged as HIDDEN so that it doesn't render anything until the
    /// user-space uses a SET_WINDOW_PROP Exec command to set it up
    fn create_window(hnd: usize) -> Result<(), Error>  {
        let mut wins = WIN_LIST.lock();
        let mut w =  Window::new(hnd);
        w.init()?;
        dbg!("create_window({:X}) - PID:{}, phys_base:{:X}, dmap_base:{:X}, \
            kmap_base:{:X} (frames: {})\n", hnd, 
            crate::sched::Task::current_pid(), w.phys_base, w.dmap_base,
            w.kmap_base,w.num_frames);
        wins.push(w); // The new window goes to the back
        Ok(())
    }

    /// Removes a window from the list
    fn destroy_window(hnd: usize) {
        // Redraw every window that has and overlap with the deceased
        // and remove it from the list!
        let w0_left;
        let w0_top;
        let w0_right;
        let w0_bottom;
        {
            let mut wins = WIN_LIST.lock();
            let Some((indx, w0)) = wins.iter_mut().enumerate()
                .find(|item| item.1.hnd == hnd) else {
                dbg!("WARN - destroy_window called on a non-existing hnd {}\n",
                    hnd);
                return; 
            };
            w0_left   = w0.left;
            w0_top    = w0.top;
            w0_right  = w0_left + w0.width;
            w0_bottom = w0_top  + w0.height;
            wins.remove(indx);

            for w in wins.iter_mut() {
                w.redraw = Self::rect_overlap(
                    (w0_left, w0_top, w0_right, w0_bottom),
                    (w.left, w.top, w.left + w.width, w.top + w.height),
                );
            }
        } // Drop the WIN_LIST lock

        let mut gui = INSTANCE.lock();
        gui.update_screen(false);
    }

    /// Reads the window properties back to the user
    fn get_window(hnd: usize,  buff: &mut [u8]) -> Result<usize, Error> {
        if buff.len() != size_of::<WindowProperties>() {
            return Err(error!(ErrorCode::InvalidArgument));
        }
        let wins = WIN_LIST.lock();
        if let Some(w) = wins.iter().find(|&w| w.hnd == hnd) {
            let result = WindowProperties {
                flags   : w.flags,
                left    : w.left,
                top     : w.top,
                width   : w.width,
                height  : w.height
            };
            copy_to_user(buff.as_mut_ptr() as usize, result);
        }
        return Err(error!(ErrorCode::InvalidHandle));
    }

    // Sets the window properties from the user input and returns the
    // virtual address of the buffer (dmap_base) that user-space must use
    fn set_window(hnd: usize, buff: &mut [u8]) -> Result<usize, Error> {
        if buff.len() != size_of::<WindowProperties>() {
            return Err(error!(ErrorCode::InvalidArgument));
        }
        let args = 
            copy_from_user::<WindowProperties>(buff.as_mut_ptr() as usize)
                            .expect("set_window failed in copy_form_user");
        let mut wins = WIN_LIST.lock();
        if let Some(w) = wins.iter_mut().find(|w| w.hnd == hnd) {
            w.flags = args.flags;
            w.left  = args.left;
            w.top   = args.top;
            w.width = args.width;
            w.height= args.height;
            return Ok(w.dmap_base);
        }
        Err(error!(ErrorCode::InvalidHandle))
    }

    /// Renders the window content
    fn update_window(hnd: usize) -> Result<usize, Error>  {
        {
            let mut wins = WIN_LIST.lock();
            let Some(w) = wins.iter_mut().find(|w| w.hnd == hnd) else {
                dbg!("WARN: update_window({:X}): not found\n", hnd);
                return Err(error!(ErrorCode::InvalidHandle));
            };
            w.redraw = true;
        } // Drop the WIN_LIST lock as update_screen needs it
        // dbg!("update_window({:X}) : ", hnd);
        let mut gui = INSTANCE.lock();
        if hnd == Self::HND_WINDOW0 {
            // Workaround: Since klog! and that ugly white/blue shell is still
            // supported in addition to this GUI module, whenever the desktop
            // window comes to focus and gets rendered, we update the entire
            // screen to get rid of the white/blue content on the framebuffer
            gui.update_screen(true);
        } else {
            gui.update_screen(false);
        }
        
        Ok(0)
    }

    fn update_mouse() {
        // TODO: this should be offloaded
        // Mark windows that overlap with the cursor
        // let (cur_left, cur_top) = Mouse::current_position();
        {
            let mut wins = WIN_LIST.lock();
            for w in wins.iter_mut() {
                w.redraw = true;
                // w.redraw = Self::rect_overlap(
                //     (cur_left, cur_top, cur_left + 24, cur_top + 24),
                //     (w.left, w.top, w.left + w.width, w.top + w.height),
                // );
            }
        } // Drop the WIN_LIST lock
        // Mark the windows
        let mut gui = INSTANCE.lock();
        gui.update_screen(false);
    }

    /// Checks if the two rectangles overlap
    /// Each rect is given as (topleft.x, topleft.y, botright.x, botright.y)
    fn rect_overlap(r1: (u32, u32, u32, u32), r2: (u32, u32, u32, u32)) -> bool{
        // The two rectangles don't overlap if
        if  r1.2 < r2.0 || // r1 is to the left of r2
            r1.0 > r2.2 || // r1 is to the right of r2
            r1.3 < r2.1 || // r1 is above r2
            r1.1 > r2.3  { // r1 is below r2
            return false;
        }
        true
    }

    /// This task goes over the list of windows and redraws the framebuffer
    /// based on updated windows in each iteration.
    /// There's a lot of optimization to be done here, e.g., calculating a list
    /// of rectangles that changed due to windows being created, destroyed,
    /// moved, resized, brought forward, etc.
    /// 
    /// However, for now, the simplest approach is picked.
    /// Iterate over the list from beginning to the end (highest z-order, i.e.,
    /// the top-most window) and apply the changes to the unified shadow fb,
    /// and then copy that over to the fb.
    /// Eventually with better graphics device support, we should be able to
    /// DMA the shadow-fb to the device or use hardware-assisted dbl-buffering.
    fn update_screen(&mut self, full_update: bool) {
        // Unlike the Windows' Graphical Context buffers that support RGBA,
        // the Framebuffer only supports RGB.
        //
        // sfb[0] and sfb[1] contain our latest copies of the framebuffer
        let sfb: [&mut[(u8,u8,u8)]; 2] = unsafe {[
            from_raw_parts_mut(
                        self.sfb_vaddr[0] as *mut (u8,u8,u8), self.pxl_cnt),
            from_raw_parts_mut(
                        self.sfb_vaddr[1] as *mut (u8,u8,u8), self.pxl_cnt)
        ]};
        // Map it to the kmap pool for our render task
        let cur_buf = self.cur_buf;
        let pre_buf = self.pre_buf;
        let mut wins = WIN_LIST.lock();
        for w in wins.iter_mut() {
            if !w.redraw {
                continue;
            }
            let wpixels = unsafe {
                from_raw_parts(w.kmap_base as *const(u8,u8,u8,u8), self.pxl_cnt)
            };
            let mut sfbx;       // Pixel index into our shadow frame buffres
            let mut wfbx = 0;   // Pixel index into the window's buffer
            for r in 0..w.height {
                sfbx = (w.top + r) as usize * self.sc_width + w.left as usize;
                for _c in 0..w.width {
                    sfb[cur_buf][sfbx] = 
                        (wpixels[wfbx].0, wpixels[wfbx].1, wpixels[wfbx].2);
                    wfbx += 1;
                    sfbx += 1;
                }
            }
            w.redraw = false;
        }
        // Draw the mouse cursor
        let (cur_x, cur_y) = Mouse::current_position();
       
        for y in 0..24.min(self.sc_height-cur_y as usize) {
            let mut sfbx = (cur_y as usize + y) * self.sc_width + cur_x as usize;
            for x in 0..24.min(self.sc_width-cur_x as usize) {
                if MOUSE_CURSOR[y][x] == 2 {
                    sfb[cur_buf][sfbx] = (30, 30, 30); // black
                } else if MOUSE_CURSOR[y][x] == 1 {
                    sfb[cur_buf][sfbx] = (240, 240, 240); // white
                }
                sfbx += 1;
            }
        }
        // Update the FB while holding the lock to throttle updates
        // coming in from the user-space. Only update the changed pixels
        let mut px = 0;
        let mut _px_count = 0;
        for r in 0..self.sc_height as u32 {
            for c in 0..self.sc_width as u32 {
                if sfb[cur_buf][px] != sfb[pre_buf][px] || full_update {
                    FrameBuffer::set_pixel(r, c, sfb[cur_buf][px]);
                    sfb[pre_buf][px] = sfb[cur_buf][px];
                    _px_count += 1;                  
                }
                px += 1;
            }
        }
        // Switch the buffers
        self.pre_buf = cur_buf;
        self.cur_buf = pre_buf;

        dbg!("{}/{} or {:.2} % of pixels updated\n",
            _px_count, self.sc_width * self.sc_height,
            _px_count as f32 / (self.sc_width * self.sc_height) as f32 * 100.0
        );
    }

}

#[repr(C, packed)]
#[derive(Debug)]
struct WindowProperties {
    flags:      u64,
    left:       u32,
    top:        u32,
    width:      u32,
    height:     u32,
}

// 0: transparent
// 1: black
// 2: white
const MOUSE_CURSOR: [[u8; 24]; 24] = [
[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 2, 2, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 2, 2, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 2, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
];


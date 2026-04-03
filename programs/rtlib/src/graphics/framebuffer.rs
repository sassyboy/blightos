//
// Direct Framebuffer Access Interface
//
use core::mem::size_of;
use crate::heap::*;
use crate::fileio::*;
use crate::graphics::RGB;
use crate::*;

pub struct Framebuffer {
    pub base_address: usize, // User-space buffer allocated on the heap
    pub height: u32,
    pub width: 	u32,
    pub bpp: 	u8,
    pub pitch: 	u32,
    // Private fields for internal use
    fb_file:    File, // Framebuffer device file
    buffer_size:usize,
    // Modified region since the last update
    modified_region: FrameBufferRect,
}

impl Framebuffer {
    const FUNC_GET_INFO:         	usize = 1;
	// Saves the current framebuffer content into a buffer provided by
	// user-space. The buffer should be large enough to hold the entire
	// framebuffer content (pitch * height * bpp/8 bytes).
	const FUNC_SAVE_FRAME:	 		usize = 2;
    const FUNC_RESTORE_FRAME: 		usize = 3;
	// Updates a rectangular region of the framebuffer with pixel data provided
	// by user-space. The user-space buffer should contain pixel data for the
	// specified rectangle (width * height * bpp/8 bytes).
	const FUNC_UPDATE_RECT: 		usize = 4;

    pub fn new() -> Option<Self> {
        // Get the framebuffer info from the kernel via syscall
        let mut instance = Self {
            base_address:   0,
            height:         0,
            width:          0,
            bpp:            0,
            pitch:          0,
            fb_file:        File::new(),
            buffer_size:    0,
            modified_region:FrameBufferRect { 
                row: u32::MAX, col: u32::MAX,
                height: 0, width: 0
            }
        };
        let fb_path = Path::from("framebuffer:/");
        let fbfopen = File::from_path(&fb_path, File::MODE_RWX);
        let Ok(fb_file) = fbfopen else {
            println!("Can't framebuffer:/ - {:?}", fbfopen.err());
            return None;
        };
        let mut info_buffer = [0u8; size_of::<FrameBufferInfo>()];
        let bytes_read = fb_file.exec(Self::FUNC_GET_INFO, &mut info_buffer)
                                                                .unwrap_or(0);
        if bytes_read == size_of::<FrameBufferInfo>() {
            // SAFETY: We trust the kernel to provide valid framebuffer info
            let fb_info: FrameBufferInfo = unsafe { 
                (info_buffer.as_ptr() as *const FrameBufferInfo).read()
            };
            instance.height = fb_info.height;
            instance.width = fb_info.width;
            instance.bpp = fb_info.bpp;
            instance.pitch = fb_info.pitch;
            // Allocate a user-space buffer for the framebuffer content
            instance.buffer_size = (instance.pitch * instance.height) as usize;
            // SAFETY: We trust the kernel to allocate a valid buffer
            instance.base_address = Malloc::malloc(instance.buffer_size) as usize;
            if instance.base_address != 0 {
                instance.fb_file = fb_file;
                return Some(instance);
            }
        }
        None
    }
    pub fn save_frame(&mut self) -> bool {
        let mut buffer = unsafe { core::slice::from_raw_parts_mut(
                            self.base_address as *mut u8, self.buffer_size) };
        return self.fb_file.exec(Self::FUNC_SAVE_FRAME, &mut buffer)
                                .unwrap_or(0) == self.buffer_size;
    }
    pub fn restore_frame(&mut self) -> bool {
        let mut buffer = unsafe { core::slice::from_raw_parts_mut(
                            self.base_address as *mut u8, self.buffer_size) };
        return self.fb_file.exec(Self::FUNC_RESTORE_FRAME, &mut buffer) 
                                .unwrap_or(0) == self.buffer_size;
    }

    pub fn set_pixel(&mut self, row: u32, col: u32, color: RGB) {
        if row < self.height && col < self.width {
            let pixel_offset = (row * self.pitch + col * (self.bpp as u32 / 8)) as usize;
            let pixel_ptr = (self.base_address + pixel_offset) as *mut u8;
            unsafe {
                // Assuming the framebuffer uses RGB format, we write the color
                // components
                core::ptr::write_volatile(pixel_ptr, color.0); // Red
                core::ptr::write_volatile(pixel_ptr.add(1), color.1); // Green
                core::ptr::write_volatile(pixel_ptr.add(2), color.2); // Blue
            }
            // Update the modified region
            self.modified_region.row = self.modified_region.row.min(row);
            self.modified_region.col = self.modified_region.col.min(col);
            self.modified_region.height = (self.modified_region.height.max(
                                            row - self.modified_region.row + 1))
                                            .min(self.height);
            self.modified_region.width = (self.modified_region.width.max(
                                            col - self.modified_region.col + 1))
                                            .min(self.width);
        }
    }

    pub fn update(&mut self) {
        if self.modified_region.height > 0 && self.modified_region.width > 0 {
            let mut update_args = FrameBufferUpdateRectArgs {
                rect: self.modified_region,
                buffer_base: self.base_address,
                buffer_size: self.buffer_size as u32,
                flags: FrameBufferUpdateRectArgs::FLAG_FULL_FRAME
            };
            // SAFETY: We trust the kernel to handle the update correctly
            let mut buffer = unsafe { 
                core::slice::from_raw_parts_mut(
                (&mut update_args as *mut FrameBufferUpdateRectArgs) as *mut u8, 
                size_of::<FrameBufferUpdateRectArgs>()
            ) };
            let _ = self.fb_file.exec(Self::FUNC_UPDATE_RECT, &mut buffer);
            // Reset modified region after update
            self.modified_region = FrameBufferRect {
                row: u32::MAX, col: u32::MAX,
                height: 0, width: 0
            };
        }
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        // Clean up resources, if necessary
        // The fb file auto closes
        if self.base_address != 0 {
            Malloc::free(self.base_address as *mut u8, self.buffer_size);
        }
    }
}


#[repr(C, packed)]
pub struct FrameBufferInfo {
	pub height: u32,
	pub width: 	u32,
	pub bpp: 	u8,
	pub pitch: 	u32
}
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FrameBufferRect {
	pub row: 	u32,
	pub col: 	u32,
	pub height: u32,
	pub width: 	u32
}
#[repr(C, packed)]
pub struct FrameBufferUpdateRectArgs {
	pub rect: 			FrameBufferRect,
	// User-buffer address containing pixel data (bpp-bytes) for the specified
	// rectangle.
	pub buffer_base: 	usize,
	pub buffer_size: 	u32,
	// Flags: Whether the buffer is a full framebuffer dump or just the updated
	// rectangle. This can help the driver optimize the update process.
	pub flags: 			u32
}
impl FrameBufferUpdateRectArgs {
	pub const FLAG_FULL_FRAME: u32 = 1;
}

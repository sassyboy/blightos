///
/// Graphics-related utilities, such as image loading and rendering.
/// 

pub type RGB = (u8, u8, u8);
pub type RGBA = (u8, u8, u8, u8);

pub struct Size2D {
    pub width:  u32, // #columns
    pub height: u32, // #rows
}

pub struct Point2D {
    pub left:   u32, // x or column
    pub top:    u32, // y or row
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub left:   u32,
    pub top:    u32,
    pub width:  u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(left: u32, top: u32, width: u32, height: u32) -> Self {
        Self { left, top, width, height }
    }

    pub fn get_position(&self) -> Point2D {
        Point2D { left: self.left, top: self.top }
    }

    pub fn get_size(&self) -> Size2D {
        Size2D { width: self.width, height: self.height }
    }

    /// Translate the rectangle's position to be relative to the given canvas
    /// rectangle. This is used to convert a widget's local position to the
    /// coordinate space
    pub fn translate(&self, canvas: &Rect) -> Rect {
        Rect {
            left: self.left + canvas.left,
            top: self.top + canvas.top,
            width: self.width,
            height: self.height,
        }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.left && x < self.left + self.width &&
        y >= self.top && y < self.top + self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorType {
    Grayscale = 0,
    RGB = 2,
    Indexed = 3,
    GrayscaleAlpha = 4,
    RGBA = 6,
}
impl ColorType {
    pub fn bytes_per_pixel(&self) -> usize {
        match *self {
            ColorType::Grayscale => 1,
            ColorType::RGB => 3,
            ColorType::Indexed => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::RGBA => 4,
        }
    }
}

#[derive(Debug)]
pub enum ImageFormat {
    Png,
    // Jpeg,
    // Bmp,
}

#[derive(Debug)]
pub struct Image {
    pub format:     ImageFormat,
    pub width:      u32,
    pub height:     u32,
    pub bit_depth:  u8,
    pub color_type: ColorType,
    pub interlaced: bool,
}

use crate::Exception;
/// Provides the basic 2D graphics funtionality
/// 
/// Draws graphics on a memory buffer that could be then written to a file such
/// as the framebuffer:/ (direct graphics access) or a window (gui:/window)
pub struct GraphicalContext {
    w:      u32,
    h:      u32,
    base:   usize, // Base virtual address of the buffer
    limit:  usize, // Size of the buffer in bytes
}
impl GraphicalContext {
    pub const fn new() -> Self {
        Self {
            w:      0,
            h:      0,
            base:   0,
            limit:  0,
        }
    }

    pub fn init(&mut self, width: u32, height: u32, addr: usize) -> Result<(), Exception> {
        self.w = width;
        self.h = height;
        self.base = addr;
        self.limit= width as usize * height as usize * 4;
        let buffer = self.base as *mut u8;
        unsafe {
            buffer.write_bytes(0, self.limit);
        }
        Ok(())
    }

    pub fn get_width(&self) -> u32 {
        self.w
    }

    pub fn get_height(&self) -> u32 {
        self.h
    }

    pub fn clear(&mut self, color: RGBA) {
        if self.base == 0 {
            return;
        }
        let pixels = self.get_buffer_mut();
        // Clear the context with the specified color
        pixels.fill(color);
    }

    /// Set the pixel at (x, y) to the specified color, blending it with the
    /// existing pixel color based on the alpha value.
    /// The coordinates are relative to the top-left corner of the context,
    /// and will be ignored if out of bounds.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: RGBA, bounds: &Rect){
        if self.base == 0 {
            return;
        }
        let w = self.w;
        let h = self.h;
        let pixels = self.get_buffer_mut();
        if x >= bounds.left && x < bounds.left + bounds.width &&  x < w &&
            y >= bounds.top && y < bounds.top + bounds.height && y < h {
            let index = (y * w + x) as usize;
            pixels[index] = Self::blend_pixels(pixels[index], color);
        }
    }

    pub fn get_pixel(&self, x: u32, y: u32) -> RGBA {
        if self.base == 0 {
            return (0, 0, 0, 0);
        }
        let w = self.w;
        let h = self.h;
        let pixels = self.get_buffer();
        if x < w && y < h {  
            let index = (y * w + x) as usize;
            pixels[index]
        } else {
            (0, 0, 0, 0) // Return transparent black for out-of-bounds
        }
    }
    pub fn draw_line(&mut self, x1: u32, y1: u32, x2: u32, y2: u32,
                        thickness: u8, color: RGBA, bounds: &Rect) {
        if thickness == 0 {
            return;
        }

        // Bresenham's line algorithm with thickness by drawing a disk at each point
        let dx = (x2 as i32 - x1 as i32).abs();
        let dy = (y2 as i32 - y1 as i32).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = if dx > dy { dx / 2 } else { -dy / 2 };
        let mut x = x1 as i32;
        let mut y = y1 as i32;

        let radius = (thickness as i32) / 2;

        loop {
            if thickness == 1 {
                // single pixel
                self.set_pixel(x as u32, y as u32, color, bounds);
            } else {
                // draw a filled disk of radius `radius` around (x, y)
                let r2 = radius * radius;
                for oy in -radius..=radius {
                    for ox in -radius..=radius {
                        if ox * ox + oy * oy <= r2 {
                            let px = x + ox;
                            let py = y + oy;
                            if px >= 0 && py >= 0 {
                                self.set_pixel(px as u32, py as u32, 
                                                                color, bounds);
                            }
                        }
                    }
                }
            }

            if x == x2 as i32 && y == y2 as i32 {
                break;
            }
            let e2 = err;
            if e2 > -dx {
                err -= dy;
                x += sx;
            }
            if e2 < dy {
                err += dx;
                y += sy;
            }
        }
    }

    pub fn fill_rect(&mut self, rect: &Rect, color: RGBA, bounds: &Rect) {
        for y in rect.top..(rect.top + rect.height) {
            for x in rect.left..(rect.left + rect.width) {
                self.set_pixel(x, y, color, bounds);
            }
        }
    }

    pub fn draw_rect(&mut self, rect: &Rect, border_width: u8, color: RGBA, 
                                                                bounds: &Rect) {
        // Draw top and bottom borders
        for x in rect.left..(rect.left + rect.width) {
            for i in 0..border_width as u32 {
                self.set_pixel(x, rect.top + i, color, bounds);
                self.set_pixel(x, rect.top + rect.height -1 - i, color, bounds);
            }
        }
        // Draw left and right borders
        for y in rect.top..(rect.top + rect.height) {
            for i in 0..border_width as u32{
                self.set_pixel(rect.left + i, y, color, bounds);
                self.set_pixel(rect.left + rect.width -1 - i, y, color, bounds);
            }
        }
    }

    pub fn draw_rect_3d(&mut self, rect: &Rect, border_width: u8, sunken: bool,
                        light_color: RGBA, dark_color: RGBA, bounds: &Rect) {
        // Draw top and bottom borders
        for x in rect.left..(rect.left + rect.width) {
            for i in 0..border_width as u32 {
                if sunken {
                    self.set_pixel(x, rect.top + i, dark_color, bounds);
                    self.set_pixel(x, rect.top + rect.height - 1 - i,
                                                        light_color, bounds);
                } else {
                    self.set_pixel(x, rect.top + i, light_color, bounds);
                    self.set_pixel(x, rect.top + rect.height - 1 - i,
                                                        dark_color, bounds);
                }
            }
        }
        // Draw left and right borders
        for y in rect.top..(rect.top + rect.height) {
            for i in 0..border_width as u32 {
                if sunken {
                    self.set_pixel(rect.left + i, y, dark_color, bounds);
                    self.set_pixel(rect.left + rect.width - 1 - i, y,
                                                        light_color, bounds);
                } else {
                    self.set_pixel(rect.left + i, y, light_color, bounds);
                    self.set_pixel(rect.left + rect.width - 1 - i, y,
                                                        dark_color, bounds);
                }
            }
        }
    }
    /// Draw a single character at the specified position using the built-in font.
     /// The character is rendered in the specified color, and will be clipped
     /// if falls outside of the provided `bounds` rectangle.
    pub fn draw_char(&mut self, c: u8, font: &Font, color: RGBA,
                                        x: u32, y: u32, bounds: &Rect) {
        let width = font.char_width(c);
        let height = font.char_height(c);
        for row in 0..height {
        	for col in 0..width {
            	if font.get_pixel(c, row, col) {
                	self.set_pixel(x + col, y + row, color, bounds);
        		}
    		}
        }
    }
    pub fn draw_text(&mut self, text: &str, font: &Font, color: RGBA,
                                        x: u32, y: u32, bounds: &Rect) {
        let mut cursor_x = x;
        for c in text.bytes() {
            self.draw_char(c, font, color, cursor_x, y, bounds);
            cursor_x += font.char_width(c) + 1; // 1 px spacing in between
        }
    }

    /// TODO: Move this to the Font struct
    pub fn measure_text_width(&self, text: &str) -> u32 {
        text.len() as u32 * 10 // Each character is 10 pixels wide in the default font
    }
    pub fn measure_text_height(&self, _text: &str) -> u32 {
        20 // Each character is 20 pixels tall in the default font
    }


    //
    // Private Helper Functions
    //
    fn get_buffer_mut(&mut self) -> &mut [RGBA] {
        unsafe {
            ::core::slice::from_raw_parts_mut(self.base as *mut RGBA,
                                            self.h as usize * self.w as usize)
        }
    }

    fn get_buffer(&self) -> &[RGBA] {
        unsafe {
            ::core::slice::from_raw_parts(self.base as *const RGBA,
                                            self.h as usize * self.w as usize)
        }
    }

    fn blend_pixels(bg: RGBA, fg: RGBA) -> RGBA {
        if fg.3 == 255 {
            // No need to blend
            return fg;
        }
        let alpha = fg.3 as f32 / 255.0;
        let ialpha= 1.0 - alpha;
        (
            (fg.0 as f32 * alpha + bg.0 as f32 * ialpha as f32) as u8,
            (fg.1 as f32 * alpha as f32 + bg.1 as f32 * ialpha as f32) as u8,
            (fg.2 as f32 * alpha as f32 + bg.2 as f32 * ialpha as f32) as u8,
            255
        )
    }

}

/// Returns the size/resolution of the current screen in pixels
pub fn screen_size() -> Option<Size2D> {
    let Some(fbi) = Framebuffer::get_framebuffer_info() else {
        return None;
    };
    Some(Size2D {
        width : fbi.width,
        height: fbi.height
    })
}

pub mod font;
pub use font::Font;
pub mod framebuffer;
pub use framebuffer::*;
pub mod png;

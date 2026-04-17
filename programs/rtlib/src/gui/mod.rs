///
/// Basic Graphical User Interface (GUI)
///

use core::any::{Any, TypeId};
use alloc::vec::Vec;
use crate::graphics::{RGBA, font::Font, framebuffer::Framebuffer};
use crate::hid::KeyboardEvent;
use crate::*;


#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub left: usize,
    pub top: usize,
    pub width: usize,
    pub height: usize,
}
impl Rect {
    pub const fn new(left: usize, top: usize, width: usize, height: usize) -> Self {
        Self { left, top, width, height }
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
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.left && x < self.left + self.width &&
        y >= self.top && y < self.top + self.height
    }
}

pub enum HorizontalAlignment {
    Left,
    Center,
    Right,
}
pub enum VerticalAlignment {
    Top,
    Middle,
    Bottom,
}

pub struct MouseEvent {
    pub x: usize,
    pub y: usize,
    pub button: u8, // 0 = left, 1 = right, 2 = middle
    pub event_type: MouseEventType,
}
pub enum MouseEventType {
    Move,
    ButtonDown,
    ButtonUp,
    WheelUp,
    WheelDown,
}

pub enum WidgetEvent {
    Focus,
    Blur,
    Mouse(MouseEvent),
    Keyboard(KeyboardEvent),
}

pub trait Widget : Any{
    fn get_position(&self) -> Rect;
    fn set_position(&mut self, pos: Rect);
    fn set_visible(&mut self, _visible: bool) {
        // By default, widgets are always visible. Override this method in the
        // widget implementation if you want to support hiding/showing the widget.
    }
    fn is_visible(&self) -> bool {
        // By default, widgets are always visible. Override this method in the
        // widget implementation if you want to support hiding/showing the widget.
        // The container widget (e.g. Window) will check this before rendering
        // the widget
        true
    }
    /// Render the widget within the bounds of the provided canvas rectangle,
    /// using the given graphical context and theme.
    /// The position of the widget is relative to the top-left corner of the
    /// canvas, and the rendering should be clipped to the canvas area.
    fn render(&mut self, gctx: &mut GraphicalContext, theme: &Theme, canvas: &Rect);
    fn handle_event(&mut self, event: WidgetEvent);

    // By default, widgets are not focusable and don't capture input events.
    // Override these methods in the widget implementation if needed.
    fn set_focus(&mut self, _focused: bool) {}
    fn is_focused(&self) -> bool { false }
    fn captures_tab(&self) -> bool { false }
    // A focusable widget that captures keyboard input should return true
    fn capture_keyboard(&self) -> bool { false }
    fn capture_mouse(&self) -> bool { false }
}

/// These methods are added to the Widget trait object to allow downcasting to
/// specific widget types (e.g. Button, Label, etc.) by container widgets
/// like Window.
impl dyn Widget {
    /// Check if the widget is of a specific type, e.g. Button, Label, etc.
    pub fn is_a<T: Widget>(&self) -> bool {
        self.type_id() == TypeId::of::<T>()
    }

    /// Downcasts the widget to a specific type. This is unsafe because
    /// it does not check if the widget is actually of the requested type.
    /// Since is_a() only works on non-mutable references, the container must
    /// perform a separate check before (with a non-mutable reference) before
    /// calling this method with a mutable reference.
    pub unsafe fn downcast_unchecked_mut<T: Widget>(&mut self) -> &mut T {
       return &mut *(self as *mut dyn Widget as *mut T);
    }

    pub fn downcast_ref<T: Widget>(&self) -> Option<&T> {
        if self.type_id() == TypeId::of::<T>() {
            unsafe {
                return Some(&*(self as *const dyn Widget as *const T));
            }
        } else {
            None
        }
    }
}

pub struct GraphicalContext {
    w:      usize,
    h:      usize,
    buf:    Vec<RGBA>,
    fb:     Option<Framebuffer>,
}
impl GraphicalContext {
    pub const fn new() -> Self {
        Self {
            w: 0,
            h: 0,
            buf: Vec::new(),
            fb: None,
        }
    }
    pub fn init(&mut self, width: usize, height: usize) {
        self.w = width;
        self.h = height;
        self.buf = Vec::with_capacity(width * height);
        for _i in 0..self.buf.capacity() {
            self.buf.push((0, 0, 0, 0)); // Initialize with transparent black
        }
    }
    pub fn get_width(&self) -> usize {
        self.w
    }
    pub fn get_height(&self) -> usize {
        self.h
    }
    pub fn clear(&mut self, color: RGBA) {
        // Clear the context with the specified color
        self.buf.fill(color);
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

    /// Set the pixel at (x, y) to the specified color, blending it with the
    /// existing pixel color based on the alpha value.
    /// The coordinates are relative to the top-left corner of the context,
    /// and will be ignored if out of bounds.
    pub fn set_pixel(&mut self, x: usize, y: usize, color: RGBA, bounds: &Rect){
        if x >= bounds.left && x < bounds.left + bounds.width &&  x < self.w &&
            y >= bounds.top && y < bounds.top + bounds.height && y < self.h {
            let index = y * self.w + x;
            self.buf[index] = Self::blend_pixels(self.buf[index], color);
        }
    }
    pub fn get_pixel(&self, x: usize, y: usize) -> RGBA {
        if x < self.w && y < self.h {
            let index = y * self.w + x;
            self.buf[index]
        } else {
            (0, 0, 0, 0) // Return transparent black for out-of-bounds
        }
    }
    pub fn draw_line(&mut self, x1: usize, y1: usize, x2: usize, y2: usize,
                        thickness: usize, color: RGBA, bounds: &Rect) {
        if thickness == 0 {
            return;
        }

        // Bresenham's line algorithm with thickness by drawing a disk at each point
        let dx = (x2 as isize - x1 as isize).abs();
        let dy = (y2 as isize - y1 as isize).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = if dx > dy { dx / 2 } else { -dy / 2 };
        let mut x = x1 as isize;
        let mut y = y1 as isize;

        let radius = (thickness as isize) / 2;

        loop {
            if thickness == 1 {
                // single pixel
                self.set_pixel(x as usize, y as usize, color, bounds);
            } else {
                // draw a filled disk of radius `radius` around (x, y)
                let r2 = radius * radius;
                for oy in -radius..=radius {
                    for ox in -radius..=radius {
                        if ox * ox + oy * oy <= r2 {
                            let px = x + ox;
                            let py = y + oy;
                            if px >= 0 && py >= 0 {
                                self.set_pixel(px as usize, py as usize, color, bounds);
                            }
                        }
                    }
                }
            }

            if x == x2 as isize && y == y2 as isize {
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
    pub fn draw_rect(&mut self, rect: &Rect, border_width: u32, color: RGBA, bounds: &Rect) {
        // Draw top and bottom borders
        for x in rect.left..(rect.left + rect.width) {
            for i in 0..border_width as usize{
                self.set_pixel(x, rect.top + i, color, bounds);
                self.set_pixel(x, rect.top + rect.height - 1 - i, color, bounds);
            }
        }
        // Draw left and right borders
        for y in rect.top..(rect.top + rect.height) {
            for i in 0..border_width as usize{
                self.set_pixel(rect.left + i, y, color, bounds);
                self.set_pixel(rect.left + rect.width - 1 - i, y, color, bounds);
            }
        }
    }

    pub fn draw_rect_3d(&mut self, rect: &Rect, border_width: u32, sunken: bool,
                        light_color: RGBA, dark_color: RGBA, bounds: &Rect) {
        // Draw top and bottom borders
        for x in rect.left..(rect.left + rect.width) {
            for i in 0..border_width as usize{
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
            for i in 0..border_width as usize{
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
                                        x: usize, y: usize, bounds: &Rect) {
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
                                        x: usize, y: usize, bounds: &Rect) {
        let mut cursor_x = x;
        for c in text.bytes() {
            self.draw_char(c, font, color, cursor_x, y, bounds);
            cursor_x += font.char_width(c) + 1; // 1 px spacing in between
        }
    }

    /// TODO: Move this to the Font struct
    pub fn measure_text_width(&self, text: &str) -> usize {
        text.len() * 10 // Each character is 10 pixels wide in the default font
    }
    pub fn measure_text_height(&self, _text: &str) -> usize {
        20 // Each character is 20 pixels tall in the default font
    }

    /// Render the context buffer to the screen at the specified position.
    pub fn render_to_screen(&mut self, origin_left: usize, origin_top: usize) {
        // Initialize the framebuffer if it hasn't been already
        if self.fb.is_none() {
            self.fb = Framebuffer::new();
        }
        // Render the context buffer to the screen at the specified position
        let Some(fb) = &mut self.fb else { return; };
        let mut pindex = 0;
        for row in 0..self.h {
            for col in 0..self.w {
                let rgba = self.buf[pindex];
                let color = (rgba.0, rgba.1, rgba.2);
                let sx = (origin_left + col) as u32;
                let sy = (origin_top + row) as u32;
                fb.set_pixel(sy, sx, color);
                pindex += 1;
            }
        }
        fb.update();
    }
}
pub mod button;
pub use button::*;
pub mod label;
pub use label::Label;
pub mod list;
pub use list::*;
pub mod menu;
pub mod textedit;
pub use textedit::TextEdit;
pub mod theme;
pub use theme::*;
pub mod window;
pub use window::*;




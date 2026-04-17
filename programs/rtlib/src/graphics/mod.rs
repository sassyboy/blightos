///
/// Graphics-related utilities, such as image loading and rendering.
/// 

pub type RGB = (u8, u8, u8);
pub type RGBA = (u8, u8, u8, u8);

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

pub mod font;
pub mod framebuffer;
pub mod png;

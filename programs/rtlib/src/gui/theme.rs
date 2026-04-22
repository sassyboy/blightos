//
//
//

use crate::graphics::{RGBA, font::Font};


pub struct Theme {
    // Colors
    pub background:     RGBA, // Main background color for windows and panels
    pub highlight:      RGBA, // Background color when hovering or selected
    pub border:         RGBA, // Border color
    pub accent:         RGBA, // Accent color for buttons, sliders, etc.
    pub text:           RGBA, // Default text color
    pub highlight_text: RGBA, // Text color when hovering or selected
    pub disabled_text:  RGBA, // Text color for disabled widgets
    // 
    pub border_width:   u8, // Border width in pixels
    pub title_bar_height: u32, // Title bar height in pixels
    //
    pub regular_font: Font, // Regular font for UI text
    pub title_font: Font,   // Font for window titles and headers
}

impl Theme {
    pub const fn new() -> Self {
        Self {
            background:     (70 , 70 , 80 , 255),
            highlight:      (80 , 80 , 90 , 255),
            border:         (0  , 0  , 0  , 255),
            accent:         (0  , 80 , 150, 255),
            text:           (160, 180, 200, 255),
            highlight_text: (255, 255, 255, 255),
            disabled_text:  (100, 100, 100, 255),
            border_width: 2,
            title_bar_height: 30,
            regular_font: Font::new(),
            title_font: Font::new(),
        }   
    }
}
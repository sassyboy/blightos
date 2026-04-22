//
// Button widget
//

use alloc::string::String;
use crate::graphics::png::PngImage;
use crate::gui::*;
use crate::hid::*;

pub enum ButtonEvent {
    OnClick(fn(&mut Button)),
    None
}

pub struct Button {
    text:           String,
    text_halign:    HorizontalAlignment,
    text_valign:    VerticalAlignment,
    pos:            Rect,
    flat:           bool,
    focused:        bool,
    visible:        bool,
    transparent_bg: bool,
    has_image:      bool,
    img_size:       Size2D,
    img_data:       Vec<RGBA>,
    // Event handling
    on_click:       ButtonEvent,
}

impl Button {
    pub fn new(text: String, pos: Rect, flat: bool) -> Self {
        Self {
            text,
            text_halign: HorizontalAlignment::Center,
            text_valign: VerticalAlignment::Middle,
            pos,
            flat,
            focused: false,
            visible: true,
            transparent_bg: false,
            has_image: false,
            img_size: Size2D { width: 0, height: 0 },
            img_data: Vec::new(),
            on_click: ButtonEvent::None,
        }
    }

    pub fn get_text(&self) -> &str {
        &self.text
    }
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }
    pub fn set_text_align(&mut self, halign: HorizontalAlignment,
                                                valign: VerticalAlignment) {
        self.text_halign = halign;
        self.text_valign = valign;
    }
    pub fn set_transparent_bg(&mut self, transparent_bg: bool) {
        self.transparent_bg = transparent_bg;
    }

    pub fn set_image_from_file(&mut self, path: &Path) {
        let Ok(mut png) = PngImage::from_path(path) else {
            self.has_image = false;
            return;
        };
        self.img_size.height = png.img.height;
        self.img_size.width  = png.img.width;
        let Ok(image) = png.decode() else {
            self.has_image = false;
            return;
        };
        self.img_data = image;
        self.has_image = true;
    
    }

    pub fn register_event(&mut self, handler: ButtonEvent) {
        match handler {
            ButtonEvent::OnClick(_) => self.on_click = handler,
            ButtonEvent::None => self.on_click = ButtonEvent::None,
        }
    }
}

impl Widget for Button {
    fn is_visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    fn get_position(&self) -> Rect {
        self.pos
    }
    fn set_position(&mut self, pos: Rect) {
        self.pos = pos;
    }

    fn render(&mut self, gctx: &mut GraphicalContext, theme: &Theme, canvas: &Rect){
        let border_width = theme.border_width;
        // Translate the button's position to be relative to gctx's coordinate
        // space based on the canvas given by the container widget (e.g. Window)
        let wrect = self.pos.translate(canvas);
        // Render the button background
        let bg_color = if self.focused {
            if self.transparent_bg {
                (theme.accent.0, theme.accent.1, theme.accent.2, 128)
            } else {
                theme.accent
            }
        } else {
            theme.highlight
        };

        if self.flat {
            if !self.transparent_bg || (self.transparent_bg && self.focused) {
                gctx.fill_rect(&wrect, bg_color, canvas);
            }
        } else {
            if !self.transparent_bg || (self.transparent_bg && self.focused) {
                gctx.fill_rect(&wrect, bg_color, canvas);
                // A simple 3D effect for the button
                let light_edge_color = theme.accent;
                let dark_edge_color = (theme.accent.0 / 2,
                                    theme.accent.1 / 2,
                                    theme.accent.2 / 2,
                                    theme.accent.3);
                gctx.draw_rect_3d(&wrect, border_width, false, 
                            light_edge_color, dark_edge_color, canvas);
            }
        }
        // Render the image if set
        if self.has_image {
            let img_left_off = (self.pos.width - self.img_size.width) / 2;
            let mut idx = 0;
            for row in 0..self.img_size.height {
                for col in 0..self.img_size.width {
                    let r = self.img_data[idx].0;
                    let g = self.img_data[idx].1;
                    let b = self.img_data[idx].2;
                    let a = self.img_data[idx].3;
                    gctx.set_pixel(wrect.left + img_left_off +col, 
                                wrect.top+row,
                                (r, g, b, a), &wrect);
                    idx += 1;
                }
            }
        }

        // Render the button text
        let text_width = theme.regular_font.text_width(self.text.as_str());
        let text_height = theme.regular_font.text_height(self.text.as_str());
        let text_x = match self.text_halign {
            HorizontalAlignment::Left => wrect.left + 5,
            HorizontalAlignment::Center => wrect.left + (wrect.width - text_width) / 2,
            HorizontalAlignment::Right => wrect.left + wrect.width - text_width - 5,
        };
        let text_y = match self.text_valign {
            VerticalAlignment::Top => wrect.top + 5,
            VerticalAlignment::Middle => wrect.top + (wrect.height - text_height) / 2,
            VerticalAlignment::Bottom => wrect.top + wrect.height - text_height - 5,
        };
        gctx.draw_text(self.text.as_str(), &theme.regular_font, theme.text,
                        text_x, text_y, canvas);
    }

    fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn is_focused(&self) -> bool {
        self.focused
    }

    fn capture_keyboard(&self) -> bool {
        true
    }

    fn handle_event(&mut self, event: WidgetEvent) {
        match event {
            WidgetEvent::Keyboard(kdb_event) => {
                // Handle keyboard events for accessibility (e.g. activate button on Enter key)
                if kdb_event.key == Key::Enter && !kdb_event.released {
                    if let ButtonEvent::OnClick(handler) = self.on_click {
                        handler(self);
                    }
                }
            },
            _ => {}
        }
    }
}

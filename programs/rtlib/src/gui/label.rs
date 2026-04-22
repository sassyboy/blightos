//
// Label widget
//

use alloc::string::String;
use crate::gui::*;

pub struct Label {
    text:       String,
    pos:        Rect,
    opaque_bg:  bool,
    bg_color:   Option<RGBA>
}

impl Label {
    pub fn new(text: String, pos: Rect, opaque_bg: bool) -> Self {
        Self {
            text,
            pos,
            opaque_bg,
            bg_color: None
        }
    }

    pub fn get_text(&self) -> &str {
        &self.text
    }
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }
    pub fn fit_to_text(&mut self, theme: &Theme) {
        // Adjust the label's width to fit the text
        let text_width = theme.regular_font.text_width(self.text.as_str());
        self.pos.width = text_width + 4; // Add some padding
        // Adjust the label's height to fit the text
        let text_height = theme.regular_font.text_height(self.text.as_str());
        self.pos.height = text_height + 4; // Add some padding
    }
    pub fn set_bg_color(&mut self, color: Option<RGBA>){
        self.bg_color = color;
    }
}

impl Widget for Label {
    fn get_position(&self) -> Rect {
        self.pos
    }
    fn set_position(&mut self, pos: Rect) {
        self.pos = pos;
    }
    
    fn render(&mut self, gctx: &mut GraphicalContext, theme: &Theme, canvas: &Rect){
        // Translate the label's position to be relative to gctx's coordinate
        // space based on the canvas given by the container widget (e.g. Window)
        let wrect = self.pos.translate(canvas);
        // Render the label background
        if self.opaque_bg {
            if self.bg_color.is_none() {
                gctx.fill_rect(&wrect, theme.background, &wrect);
            } else {
                gctx.fill_rect(&wrect, self.bg_color.unwrap(), &wrect);
            }
            
        }
        // Render the label text
        // Todo: Implement text clipping based on the canvas bounds
        // Todo: Implement text alignment (left, center, right) 
        //       and vertical alignment (top, middle, bottom)
        gctx.draw_text(self.text.as_str(), &theme.regular_font, theme.text,
                                                wrect.left, wrect.top, &wrect);
    }

    fn handle_event(&mut self, _event: WidgetEvent) {
        // Labels don't handle any events for now
        
    }
}
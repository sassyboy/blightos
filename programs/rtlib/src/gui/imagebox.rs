//
// Loads an image from disk into memory and renders it
//

use crate::graphics::png::PngImage;
use crate::gui::*;

pub struct ImageBox {
    pos:        Rect,
    img_size:   Size2D,
    img_data:   Vec<RGBA>,
    loaded:     bool
}

impl ImageBox {
    pub const fn new(pos: Rect) -> Self {
        Self {
            pos,
            img_size: Size2D { width: 0, height: 0 },
            img_data: Vec::new(),
            loaded: false
        }
    }

    pub fn load_from_file(&mut self, path: &Path) {
        let Ok(mut png) = PngImage::from_path(path) else {
            self.loaded = false;
            return;
        };
        self.img_size.height = png.img.height;
        self.img_size.width  = png.img.width;
        let Ok(image) = png.decode() else {
            self.loaded = false;
            return;
        };
        self.img_data = image;
        self.loaded = true;
    }


}

impl Widget for ImageBox {
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

        // Just render a text in the center/middle if no image has been loaded
        if !self.loaded {
            gctx.fill_rect(&wrect, theme.background, &wrect);
            let tw = gctx.measure_text_width("No Image");
            let th = gctx.measure_text_height("No Image");
            gctx.draw_text("No Image!", &theme.regular_font, theme.text,
                            wrect.left + (wrect.width - tw) / 2,
                            wrect.top + (wrect.height - th) / 2,
                            &wrect);
            return;
        }
        // Render the loaded image
        // Todo: Implement tiling or stretching...
        let mut idx = 0;
        for row in 0..self.img_size.height {
            for col in 0..self.img_size.width {
                let r = self.img_data[idx].0;
                let g = self.img_data[idx].1;
                let b = self.img_data[idx].2;
                let a = 255;
                gctx.set_pixel(wrect.left+col, 
                                wrect.top+row,
                                (r, g, b, a), &wrect);
                idx += 1;
            }
        }
        
    }

    fn handle_event(&mut self, _event: WidgetEvent) {
        // Labels don't handle any events for now
        
    }
}
//
// List widget
//

use alloc::string::String;
use alloc::vec::Vec;
use crate::gui::*;
use crate::hid::*;

pub enum ListViewEvent {
    OnKeyPress(fn(&mut ListView, KeyboardEvent)),
    None
}

pub struct ListView {
    pos:                Rect,
    opaque_bg:          bool,
    flat:               bool,
    focused:            bool,
    columns:            Vec<String>,
    items:              Vec<Vec<String>>,
    column_widths:      Vec<u32>,
    row_height:         u32,
    // List view state
    top_visible_row:    usize,
    num_visible_rows:   usize,
    selected_row:       usize,
    // Event handling
    on_key_press:       ListViewEvent,
}

impl ListView {
    pub fn new(pos: Rect, opaque_bg: bool, flat: bool) -> Self {
        Self {
            pos,
            opaque_bg,
            flat,
            focused: false,
            columns: Vec::new(),
            items: Vec::new(),
            column_widths: Vec::new(),
            row_height: 30, // Default row height
            top_visible_row: 0,
            num_visible_rows: 0,
            selected_row: 0,
            on_key_press: ListViewEvent::None,
        }
    }

    pub fn add_column(&mut self, column_name: String, width: u32) {
        self.columns.push(column_name);
        self.column_widths.push(width);
    }

    pub fn add_item(&mut self, item: Vec<String>) {
        if item.len() != self.columns.len() {
            // Invalid item, number of fields must match number of columns
            return;
        }
        self.items.push(item);
    }

    pub fn clear_items(&mut self) {
        self.top_visible_row = 0;
        self.selected_row = 0;
        self.items.clear();
    }

    pub fn get_selected_item(&self) -> Option<&Vec<String>> {
        if self.selected_row < self.items.len() && self.items.len() > 0 {
            Some(&self.items[self.selected_row])
        } else {
            None
        }
    }

    pub fn register_event(&mut self, handler: ListViewEvent) {
        match handler {
            ListViewEvent::OnKeyPress(_) => self.on_key_press = handler,
            ListViewEvent::None => self.on_key_press = ListViewEvent::None,
        }
    }

    fn move_cursor_up(&mut self) {
        if self.selected_row > 0 {
            self.selected_row -= 1;
            if self.selected_row < self.top_visible_row {
                self.top_visible_row = self.selected_row;
            }
        }
    }

    fn move_cursor_down(&mut self) {
        if self.selected_row + 1 < self.items.len() {
            self.selected_row += 1;
            if self.selected_row >= self.top_visible_row + self.num_visible_rows {
                self.top_visible_row = self.selected_row - self.num_visible_rows + 1;
            }
        }
    }

    fn handle_keyboard_event(&mut self, kbde: KeyboardEvent) {
        if kbde.released {
            return;
        }
        match kbde.key {
            Key::PageUp => {
                for _ in 0..self.num_visible_rows {
                    self.move_cursor_up();
                }
            },
            Key::PageDown => {
                for _ in 0..self.num_visible_rows {
                    self.move_cursor_down();
                }
            },
            Key::Up => {
                self.move_cursor_up();
            },
            Key::Down => {
                self.move_cursor_down();
            },
            _ => { }
        }
        // Call the user-defined callback for key press events (if any)
        if let ListViewEvent::OnKeyPress(handler) = self.on_key_press {
            handler(self, kbde);
        }
    }

}

impl Widget for ListView {
    fn get_position(&self) -> Rect {
        self.pos
    }
    fn set_position(&mut self, pos: Rect) {
        self.pos = pos;
    }
    
    fn capture_keyboard(&self) -> bool {
        true
    }

    fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
    }
    fn is_focused(&self) -> bool {
        self.focused
    }
    fn captures_tab(&self) -> bool {
        false
    }

    fn render(&mut self, gctx: &mut GraphicalContext, theme: &Theme, canvas: &Rect){
        let border_width = theme.border_width as usize;
        // Translate the text edit's position to be relative to gctx's coordinate
        // space based on the canvas given by the container widget (e.g. Window)
        let wrect = self.pos.translate(canvas);
        // Render the text edit background
        if self.opaque_bg {
            gctx.fill_rect(&wrect, theme.highlight, canvas);
        }

        // Render the widget's border
        if self.flat {
            gctx.draw_rect(&wrect, theme.border_width, theme.border, canvas);
        } else {
            // Draw a sunken border effect
            let light_edge_color;
            let dark_edge_color;
            if self.is_focused() {
                light_edge_color = theme.accent;
                dark_edge_color = (theme.accent.0 / 2,
                                    theme.accent.1 / 2,
                                    theme.accent.2 / 2,
                                    theme.accent.3);
            } else {
                light_edge_color = theme.border;
                dark_edge_color = (theme.border.0 / 2,
                                    theme.border.1 / 2,
                                    theme.border.2 / 2,
                                    theme.border.3);
            }
            gctx.draw_rect_3d(&wrect, border_width as u32, true, 
                            light_edge_color, dark_edge_color, canvas);
        }

        // Render the column headers
        let mut x = wrect.left + border_width;
        for (i, column) in self.columns.iter().enumerate() {
            let col_width = self.column_widths[i];
            let header_rect = Rect {
                left: x,
                top: wrect.top + border_width,
                width: col_width as usize,
                height: self.row_height as usize,
            };
            gctx.fill_rect(&header_rect, theme.background, canvas);
            gctx.draw_rect(&header_rect, 1, theme.border, canvas);
            gctx.draw_text(column.as_str(), &theme.regular_font,
                            theme.highlight_text, header_rect.left + 5, 
                            header_rect.top+ 3, canvas);
            x += col_width as usize;
        }

        //
        // Render the items
        //
        let lstrect = Rect {
            left: wrect.left + border_width,
            top: wrect.top + border_width + self.row_height as usize,
            width: wrect.width - 2 * border_width,
            height: wrect.height - 2 * border_width - self.row_height as usize,
        };
        self.num_visible_rows = (lstrect.height) / (self.row_height as usize);
        for i in 0..self.num_visible_rows {
            let item_index = self.top_visible_row + i;
            if item_index >= self.items.len() {
                break;
            }
            let item = &self.items[item_index];
            let mut x = lstrect.left;
            let y = lstrect.top + i * (self.row_height as usize);
            for (j, field) in item.iter().enumerate() {
                let col_width = self.column_widths[j];
                let field_rect = Rect {
                    left: x,
                    top: y,
                    width: col_width as usize,
                    height: self.row_height as usize,
                };
                // Highligh the selected row if focused
                if item_index == self.selected_row && self.is_focused() {
                    gctx.fill_rect(&field_rect, theme.accent, &lstrect);
                    gctx.draw_text(field.as_str(), &theme.regular_font,
                                theme.highlight_text, field_rect.left + 5,
                                field_rect.top + 3, &lstrect);
                } else {
                    gctx.draw_text(field.as_str(), &theme.regular_font,
                                theme.text, field_rect.left + 5,
                                field_rect.top + 3, &lstrect);

                }
                
                x += col_width as usize;
            }
        }
    }

    fn handle_event(&mut self, event: WidgetEvent) {
        match event {
            WidgetEvent::Keyboard(kbde) => {
                self.handle_keyboard_event(kbde);    
            }
            _ => {  }
        }
    }
}
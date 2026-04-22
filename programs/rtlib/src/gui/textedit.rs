//
// TextEdit widget
// Provides a simple multi-line text editor with basic editing capabilities.
//

use alloc::string::String;
use alloc::format;
use crate::gui::*;
use crate::hid::*;

pub struct TextEdit {
    pos:                Rect,
    opaque_bg:          bool,
    flat:               bool,
    focused:            bool,
    //
    pub line_numbers:   bool,
    pub line_wrapping:  bool,
    pub read_only:      bool,
    pub tab_size:       u8,
    text:               Vec<String>, // A vector of lines
    // Cursor position in terms of line and column indices
    cursor_line:        usize,
    cursor_col:         usize,
    // Visible area of the text
    visible_top_line:   usize,
    visible_left_col:   usize,
    num_visible_lines:  usize
}

impl TextEdit {
    pub fn new(pos: Rect, opaque_bg: bool, flat: bool) -> Self {
        Self {
            text: Vec::new(),
            pos,
            opaque_bg,
            flat,
            line_numbers:       true, // TODO: change to false after testing
            line_wrapping:      true,
            focused:            false,
            read_only:          false,
            tab_size:           4,
            cursor_line:        0,
            cursor_col:         0,
            visible_top_line:   0,
            visible_left_col:   0,
            num_visible_lines:  0,
        }
    }

    pub fn get_text(&self) -> String {
        self.text.join("\n")
    }
    pub fn set_text(&mut self, text: String) {
        self.text = text.split('\n').map(String::from).collect();
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.visible_top_line = 0;
        self.visible_left_col = 0;
    }
    pub fn get_cursor_position(&self) -> (usize, usize) {
        (self.cursor_line, self.cursor_col)
    }
    
    fn calc_num_visible_lines(&self, theme: &Theme) -> u32 {
        let char_height = theme.regular_font.char_height(b' ');
        let line_spacing = theme.regular_font.line_spacing;
        (self.pos.height - 4) / (char_height + line_spacing)
    }

    fn move_cursor_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            let line_len = self.text[self.cursor_line].len();
            self.cursor_col = self.cursor_col.min(line_len);
            if self.cursor_line < self.visible_top_line {
                self.visible_top_line -= 1;
            }
        }
    }
    fn move_cursor_down(&mut self) {
        if self.cursor_line < self.text.len() - 1 {
            self.cursor_line += 1;
            let line_len = self.text[self.cursor_line].len();
            self.cursor_col = self.cursor_col.min(line_len);
            if self.cursor_line >=
                self.visible_top_line + self.num_visible_lines {
                self.visible_top_line += 1;
            }
        } else {
            // Move cursor to end of last line if we are on the last line
            self.cursor_col = self.text[self.cursor_line].len();
        }
    }
    fn move_cursor_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.text[self.cursor_line].len();
            if self.cursor_line < self.visible_top_line {
                self.visible_top_line -= 1;
            }
        }
    }
    fn move_cursor_right(&mut self) {
        let line_len = self.text[self.cursor_line].len();
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_line < self.text.len() - 1 {
            self.cursor_line += 1;
            self.cursor_col = 0;
            if self.cursor_line >=
                self.visible_top_line + self.num_visible_lines {
                self.visible_top_line += 1;
            }
        }
    }

    fn handle_keyboard_event(&mut self, kbde: KeyboardEvent) {
        if kbde.released {
            return;
        }
        match kbde.key {
            Key::PageUp => {
                for _ in 0..self.num_visible_lines {
                    self.move_cursor_up();
                }
            },
            Key::PageDown => {
                for _ in 0..self.num_visible_lines {
                    self.move_cursor_down();
                }
            },
            Key::Up => {
                self.move_cursor_up();
            },
            Key::Down => {
                self.move_cursor_down();
            },
            Key::Left => {
                self.move_cursor_left();
            },
            Key::Right => {
                self.move_cursor_right();
            },
            Key::Backspace => {
                if self.read_only {
                    return;
                }
                if self.cursor_col > 0 {
                    self.text[self.cursor_line].remove(self.cursor_col - 1);
                    self.move_cursor_left();
                } else if self.cursor_line > 0 {
                    let current_line = self.text.remove(self.cursor_line);
                    let prev_line = &mut self.text[self.cursor_line - 1];
                    let prev_line_len = prev_line.len();
                    prev_line.push_str(&current_line);
                    self.cursor_col = prev_line_len;
                    self.move_cursor_up();
                }
            },
            Key::Delete => {
                if self.read_only {
                    return;
                }
                if self.cursor_col < self.text[self.cursor_line].len() {
                    self.text[self.cursor_line].remove(self.cursor_col);
                } else if self.cursor_line < self.text.len() - 1 {
                    let next_line = self.text.remove(self.cursor_line + 1);
                    self.text[self.cursor_line].push_str(&next_line);
                }
            },
            Key::Enter => {
                if self.read_only {
                    return;
                }
                let line = &mut self.text[self.cursor_line];
                let new_line = line.split_off(self.cursor_col);
                self.text.insert(self.cursor_line + 1, new_line);
                self.move_cursor_down();
                self.cursor_col = 0;
            },
            _ => {
                if self.read_only || kbde.to_ascii().is_none() {
                    return;
                }
                let c = kbde.to_ascii().unwrap() as char;
                let line = &mut self.text[self.cursor_line];
                line.insert(self.cursor_col, c);
                self.move_cursor_right();
            }
        }
    }
    fn draw_cursor(&self, gctx: &mut GraphicalContext, theme: &Theme,
                    cur_line: usize, char_left: u32, char_top: u32,
                                            char_height: u32, trect: &Rect) {
        if self.is_focused() && cur_line == self.cursor_line {
            let mut color = theme.accent;
            color.3 = 180; // Make the cursor semi-transparent
            gctx.fill_rect(&Rect::new(char_left, char_top, 3, char_height),
                                                                color, &trect);
        }
    }
}

impl Widget for TextEdit {
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
        true
    }

    fn render(&mut self, gctx: &mut GraphicalContext, theme: &Theme, canvas: &Rect){
        let border_width = theme.border_width as u32;
        // Translate the text edit's position to be relative to gctx's coordinate
        // space based on the canvas given by the container widget (e.g. Window)
        let wrect = self.pos.translate(canvas);
        // Render the text edit background
        if self.opaque_bg {
            gctx.fill_rect(&wrect, theme.highlight, canvas);
        }
        // The area where the text will be rendered depends on whether line
        // numbers are enabled.
        let trect = if self.line_numbers {
            let digits = self.text.len().ilog10() as u32 + 1;
            let margin_width = theme.regular_font.char_width(b'0') * (digits+1);
            let lno_rect = Rect {
                left: wrect.left + border_width,
                top: wrect.top + border_width,
                width: margin_width,
                height: wrect.height,
            };
            gctx.fill_rect(&lno_rect, theme.background, canvas);
            Rect {
                left: lno_rect.left + margin_width,
                top: lno_rect.top,
                width: wrect.width - margin_width - border_width * 2,
                height: wrect.height - border_width * 2,
            }
        } else {
            Rect {
                left: wrect.left + border_width,
                top: wrect.top + border_width,
                width: wrect.width - border_width * 2,
                height: wrect.height - border_width * 2,
            }
        };
        

        // Render the widget's border
        if self.flat {
            gctx.draw_rect(&wrect, theme.border_width, theme.border, canvas);
        } else {
            let light_edge_color;
            let dark_edge_color;
            // Draw a sunken border effect
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
            gctx.draw_rect_3d(&wrect, border_width as u8, true, 
                            light_edge_color, dark_edge_color, canvas);
        }

        // For debugging - draw the text rendering area
        // gctx.draw_line(trect.left, trect.top, 
        //                  trect.left, trect.top+trect.height,
        //                  1, (0, 255, 0, 255), canvas);
        // gctx.draw_line(trect.left + trect.width, trect.top,
        //                 trect.left + trect.width, trect.top+trect.height,
        //                 1, (255, 0, 0, 255), canvas);

        // Render the text edit content within the visible area of the widget
        let cspace = theme.regular_font.kerning;
        let lspace = theme.regular_font.line_spacing;
        let mut char_top = trect.top + lspace;
        let char_height = theme.regular_font.char_height(b' ');
        self.num_visible_lines = self.calc_num_visible_lines(theme) as usize;
        for (lno,line) in self.text.iter().enumerate() {
            // Only render lines that are within the visible area based on the
            // current scroll position
            if lno < self.visible_top_line {
                continue;
            }
            if lno >= self.visible_top_line + self.num_visible_lines {
                break;
            }

            // Render the line number column if enabled.
            // The current line is highlighted with a different color.
            if self.line_numbers {
                let line_num_str = format!("{:>width$} ", lno + 1,
                            width = (self.text.len().ilog10() as usize + 1));
                let line_number_color = if lno == self.cursor_line {
                    theme.highlight_text
                } else {
                    theme.disabled_text
                };
                gctx.draw_text(&line_num_str, &theme.regular_font,
                                line_number_color, 
                                wrect.left + 2, char_top, canvas);
            }

            // Replace tabs with a number of spaces equal to tab_size for
            // visualization.
            if self.line_wrapping {
                // Wrap the line around if it exceeds the visible width.
                let mut char_left = trect.left + cspace;
                if line.is_empty() {
                    // Put the cursor at the beginning of the empty line
                    self.draw_cursor(gctx, theme, lno, char_left, char_top,
                                                        char_height, &trect);
                }
                for (col, b) in line.as_bytes().iter().enumerate() {
                    if char_top + char_height > trect.top + trect.height {
                        break; // Stop rendering if we exceed the visible area
                    }
                    // Handle tabs
                    let char_ascii;
                    let char_width;
                    if *b != b'\t' {
                        char_ascii = *b;
                        char_width = theme.regular_font.char_width(*b) + cspace
                    } else {
                        char_ascii = b' ';
                        char_width = self.tab_size as u32 *
                        theme.regular_font.char_width(b' ') + cspace
                    };
                    if char_left + char_width < trect.left + trect.width {
                        gctx.draw_char(char_ascii, &theme.regular_font,
                                theme.text, char_left, char_top, &trect);
                    } else {
                        // Move to next line if we exceed the visible width
                        char_left = trect.left + cspace;
                        char_top += char_height + lspace;
                        self.num_visible_lines -= 1; 
                        gctx.draw_char(char_ascii, &theme.regular_font,
                                theme.text, char_left, char_top, &trect);
                    }
                    
                    // Draw the cursor if the widget is focused and the cursor
                    // is on this line and column.
                    if self.cursor_col == col {
                        self.draw_cursor(gctx, theme, lno, char_left, char_top,
                                                        char_height, &trect);
                    } else if self.cursor_col == line.len() && 
                                                    col == line.len() - 1 {
                        // Draw the cursor at end of line
                        self.draw_cursor(gctx, theme, lno,
                                char_left + char_width, char_top, char_height,
                                                                        &trect);
                    }
                    char_left += char_width;
                }
            } else {
                // Todo - Add support for horizontal scrolling
                // gctx.draw_text(&vline, &theme.regular_font, theme.text,
                //             trect.left, char_top, &trect);
            }

            char_top += char_height + theme.regular_font.line_spacing;
            if char_top > trect.top + trect.height {
                break; // Stop rendering if we exceed the visible area
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

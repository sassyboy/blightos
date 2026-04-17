use core::time::Duration;

///
/// Window Widget
/// 

use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::string::String;
use crate::gui::*;
use crate::hid::*;
use crate::task::Task;

pub enum WindowEvent {
    OnKeyPress(fn(&mut Window, KeyboardEvent)),
    None,
}

#[derive(PartialEq, Clone, Copy)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Closing,
}

pub struct Window {
    title:          String,
    title_align:    HorizontalAlignment,
    theme:          Theme,
    pos:            Rect,
    flags:          u32,
    gctx:           GraphicalContext,
    widgets:        Vec<Box<dyn Widget>>,
    focused_widget: Option<usize>,
    //
    active:         bool,
    // Event handling
    state:          WindowState,
    kbd:            Keyboard,
    on_key_press:   WindowEvent,
}

impl Window {
    pub const FLAGS_BORDERLESS:     u32 = 0x1;
    pub const FLAGS_RESIZABLE:      u32 = 0x2;
    pub const FLAGS_MOVABLE:        u32 = 0x4;
    pub const FLAGS_CLOSEABLE:      u32 = 0x8;
    pub const FLAGS_MINIMIZABLE:    u32 = 0x10;
    pub const FLAGS_MAXIMIZABLE:    u32 = 0x20;
    pub const FLAGS_ALWAYS_ON_TOP:  u32 = 0x40;

    pub const fn new() -> Self {
        Self {
            title: String::new(),
            title_align: HorizontalAlignment::Center,
            theme: Theme::new(),
            pos: Rect::new(0, 0, 0, 0),
            flags: 0,
            gctx: GraphicalContext::new(),
            widgets: Vec::new(),
            focused_widget: None,
            active: true,
            state: WindowState::Normal,
            kbd: Keyboard::new(),
            on_key_press: WindowEvent::None
        }
    }
    pub fn init(&mut self, title: String, pos: Rect) {
        self.title = title;
        self.pos = pos;
        self.gctx.init(pos.width, pos.height);
    }

    pub fn get_title(&self) -> &str {
        &self.title
    }
    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    pub fn get_flags(&self) -> u32 {
        self.flags
    }
    pub fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }

    pub fn get_position(&self) -> Rect {
        self.pos
    }
    pub fn set_position(&mut self, pos: Rect) {
        self.pos = pos;
    }

    pub fn register_event(&mut self, handler: WindowEvent) {
        match handler {
            WindowEvent::OnKeyPress(_) => self.on_key_press = handler,
            _ => {}
        }
    }

    pub fn add_widget(&mut self, widget: Box<dyn Widget>) {
        self.widgets.push(widget);
        // If there is no focused widget and this one is focusable, set it as
        //the focused widget
         if self.focused_widget.is_none() &&
                            self.widgets.last().unwrap().capture_keyboard() {
            self.focused_widget = Some(self.widgets.len() - 1);
            self.widgets.last_mut().unwrap().set_focus(true);
        }
    }

    pub fn remove_widget(&mut self, index: usize) {
        if index < self.widgets.len() {
            self.widgets.remove(index);
        }
    }

    pub fn borrow_widget_ref<T: Widget + 'static>(&self, index: usize) 
                                                                -> Option<&T> {
        match self.widgets.get(index) {
            Some(widget) => widget.downcast_ref::<T>(),
            None => None,
        }
    }

    pub fn borrow_widget_mut<T: Widget + 'static>(&mut self, index: usize) 
                                                            -> Option<&mut T> {
        if let Some(widget) = self.widgets.get(index) {
            if !widget.is_a::<T>() {
                // Widget exists but is not of the requested type
                return None;
            }
        } else{
            // Index out of bounds
            return None;
        }
        // Now it's safe to downcast without checking the type or index
        unsafe { 
            Some(self.widgets.get_mut(index).unwrap()
                                                .downcast_unchecked_mut::<T>())
        }
    }


    pub fn get_state(&self) -> WindowState {
        self.state
    }
    pub fn set_state(&mut self, state: WindowState) {
        self.state = state;
    }
    pub fn close(&mut self) {
        self.kbd.stop_listening();
        self.state = WindowState::Closing;
    }

    /// Prepares the window to receive input from any attached keyboard and
    /// mouse, brings the window to focus, and renders the window.
    pub fn show(&mut self, event_loop: fn(&mut Window)->bool) {
        if let Err(e) = self.kbd.start_listening() {
            println!("Error opening the kbd: device: {:?}", e);
        }
        self.render();
        // Wait for any pending key release and then clear the queue
        Task::sleep(Duration::from_millis(250));
        self.kbd.flush_events();
        // Process the events in a loop
        loop {
            if !event_loop(self) || self.state == WindowState::Closing {
                break;
            }
            Task::yield_now();
        }
    }

    pub fn show_modal_window(&mut self, win: &mut Window,
                                            event_loop: fn(&mut Window)->bool) {
        self.active = false;
        self.render();
        
        win.show(event_loop);
        self.kbd.flush_events();
        self.active = true;
        self.render();
    }

    pub fn render(&mut self) {
        // The co-ordinates here are relative to the window (not the screen)
        let wrect = Rect {
            left: 0,
            top: 0,
            width: self.pos.width,
            height: self.pos.height,
        };
        // Render the window background and border
        self.gctx.fill_rect(&wrect, self.theme.background, &wrect);
        if self.flags & Self::FLAGS_BORDERLESS == 0 {
            self.gctx.draw_rect(&wrect, self.theme.border_width, 
                                                    self.theme.accent, &wrect);
        }
        // Render the title bar``
        let title_bar_height = 30;
        let title_bar_rect = Rect {
            left: 0,
            top: 0,
            width: self.pos.width,
            height: title_bar_height,
        };
        let title_text_x = match self.title_align {
            HorizontalAlignment::Left => 5,
            HorizontalAlignment::Center => (self.pos.width -
                self.theme.title_font.text_width(self.title.as_str())) / 2,
            HorizontalAlignment::Right => self.pos.width -
                self.theme.title_font.text_width(self.title.as_str()) - 5,
        };
        let title_text_y = (title_bar_height -
                                self.theme.title_font.char_height(b' ')) / 2;
        let title_text_color;
        let title_bar_color;
        if self.active {
            title_bar_color = self.theme.accent;
            title_text_color = self.theme.highlight_text;
        } else {
            title_bar_color = self.theme.highlight;
            title_text_color = self.theme.disabled_text;
        }
        self.gctx.fill_rect(&title_bar_rect, title_bar_color, &wrect);
        self.gctx.draw_text(&self.title, &self.theme.title_font,
                        title_text_color, title_text_x, title_text_y, &wrect);

        // Render child widgets
        let widgets_canvas = Rect {
            left: 0 + self.theme.border_width as usize,
            top: title_bar_height,
            width: self.pos.width - 2 * self.theme.border_width as usize,
            height: self.pos.height - title_bar_height,
        };
        for widget in self.widgets.iter_mut() {
            if widget.is_visible() {
                widget.render(&mut self.gctx, &self.theme, &widgets_canvas);
            }
        }

        // Transfer everything to the screen via the graphics context
        self.gctx.render_to_screen(self.pos.left, self.pos.top);
    }

    /// The owner task of the window should call this function in a loop to
    /// process events and update the window as needed.
    /// process_event() will fetch events from various event queues and dispatch
    /// them to widgets as needed. The widgets will in turn update their state
    /// and execute any user-defined callbacks as needed.
    /// 
    /// Returns true if the window should be redrawn
    pub fn process_event(&mut self) -> bool {
        let mut redraw_needed = false;
        // Scan the keyboard event queue
        let kbd_events = self.kbd.fetch_events();
        if !self.active {
            return false; // Ignore the events if not the active window
        }
        for key_event in kbd_events {
            if self.process_keyboard_event(key_event) {
                redraw_needed = true;
            }
        }

        redraw_needed
    }

    fn process_keyboard_event(&mut self, key_event: KeyboardEvent) -> bool {
        // Process the event by the window's handler first
        if let WindowEvent::OnKeyPress(handler) = self.on_key_press {
            handler(self, key_event);
        }

        if key_event.key == Key::Tab && key_event.released {
            // Tab key to switch focus between widgets, and pass the event to
            // the focused widget iff the currently focused widget doesn't
            // capture the Tab key itself
            if let Some(focused_index) = self.focused_widget {
                if !self.widgets[focused_index].captures_tab() {
                    self.focus_next_widget();
                    return true;
                }
            } else {
                // No focused widget: try to focus the first focusable one
                self.focus_next_widget();
                return true;
            }
        } else if key_event.key == Key::Escape && key_event.released {
            // Escape key to unfocus any focused widget
            if let Some(focused_index) = self.focused_widget {
                self.widgets[focused_index].set_focus(false);
                self.focused_widget = None;
                return true;
            }
        }
        // Not Tab: Send the key event to the focused widget (if any)
        if let Some(focused_index) = self.focused_widget {
            self.widgets[focused_index].handle_event(
                                            WidgetEvent::Keyboard(key_event));
            return true;
        }
        false
    }

    fn focus_next_widget(&mut self) {
        if let Some(focused_index) = self.focused_widget {
            let mut next_index = (focused_index + 1) % self.widgets.len();
            // Loop until we find a focusable widget or come back to the
            // original one
            while (!self.widgets[next_index].capture_keyboard() ||
                     !self.widgets[next_index].is_visible()) &&
                    next_index != focused_index {
                next_index = (next_index + 1) % self.widgets.len();
            }
            if self.widgets[next_index].capture_keyboard() {
                self.focused_widget = Some(next_index);
                self.widgets[focused_index].set_focus(false);
                self.widgets[next_index].set_focus(true);
            } else {
                self.focused_widget = None;
            }
        } else {
            // No widget is currently focused, so try to focus the first
            // focusable widget
            for (i, widget) in self.widgets.iter_mut().enumerate() {
                if widget.capture_keyboard() {
                    self.focused_widget = Some(i);
                    widget.set_focus(true);
                    break;
                }
            }
        }
    }
}
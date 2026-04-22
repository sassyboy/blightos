///
/// Basic Graphical User Interface (GUI)
///

use core::any::{Any, TypeId};
use alloc::vec::Vec;
use crate::fileio::{File, Path};
use crate::graphics::*;
use crate::hid::KeyboardEvent;
use crate::*;


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

pub mod button;
pub use button::*;
pub mod imagebox;
pub use imagebox::*;
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




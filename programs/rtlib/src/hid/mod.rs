//
// Human Input Device Interface
//
use alloc::vec::Vec;
use crate::Exception;
use crate::fileio::{Path, File};
use crate::*;
///
/// The Keyboard struct provides an interface for keyboard input via the VFS
/// mount-point "kbd:/"
/// It starts listening/capturing keycodes as soon as it's initialized,
/// and stops when dropped.
/// 
pub struct Keyboard {
    kbd_file: File,
    shift_pressed: bool,
    ctrl_pressed: bool,
    alt_pressed: bool,
    expect_extended_code: bool,
}
impl Keyboard {
    pub const fn new() -> Self {
        Self {
            kbd_file:               File::new(),
            shift_pressed:          false,
            ctrl_pressed:           false,
            alt_pressed:            false,
            expect_extended_code:   false,
        }
    }

    pub fn start_listening(&mut self) -> Result<(), Exception> {
        if self.kbd_file.is_open() {
            // Already listening
            return Ok(());
        }
        let path = Path::from("kbd:/all");
        self.kbd_file.open(&path, File::MODE_READ | File::MODE_STREAM)?;
        Ok(())
    }

    pub fn stop_listening(&mut self) {
        self.expect_extended_code = false;
        self.kbd_file.close();
    }

    pub fn fetch_events(&mut self) -> Vec<KeyboardEvent> {
        let mut events = Vec::new();
        // Read the first byte of the keycode
        let mut buf = [0u8; 128];
        let mut len: usize = 0;
        if self.kbd_file.is_open() {
            if let Ok(l) = self.kbd_file.read(&mut buf) {
                len = l;
            } else {
                // Error reading from the keyboard file
                println!("E1");
                len = 0;
            }
        } 
        for raw_code in buf[0..len].iter() {
            let mut keycode = *raw_code as u16;
            if self.expect_extended_code {
                // In the middle of an extended keycode in the last fetch?
                keycode = 0xE000 | keycode;
                self.expect_extended_code = false;
            } else if keycode == 0xE0 {
                // Wait for the next byte to complete the extended keycode
                self.expect_extended_code = true;
                continue; 
            }
            // Extract the KeyPress/Release bit from the code and clear it
            let released = (keycode & 0x80) != 0;
            keycode &= 0xFF7F; 
            // Update modifier states if the key is a modifier key
            match keycode {
                0x2A | 0x36 => self.shift_pressed = !released,
                0x1D | 0xE01D => self.ctrl_pressed = !released,
                0x38 | 0xE038 => self.alt_pressed = !released,
                _ => {}
            }
            // Create a KeyboardEvent with the keycode and modifier states
            let key = Key::from(keycode);
            let mut modif: u8 = 0;
            if self.shift_pressed { modif |= KeyboardEvent::MODIFIER_SHIFT; }
            if self.ctrl_pressed  { modif |= KeyboardEvent::MODIFIER_CTRL; }
            if self.alt_pressed   { modif |= KeyboardEvent::MODIFIER_ALT; }
            events.push(KeyboardEvent { key, released, modifiers: modif });
        }
        events
    }

    pub fn flush_events(&mut self) {
        let _ = self.fetch_events();
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
#[repr(u16)]
pub enum Key {
    // Standard keyboard keys (KeyCode & 0x7F)
    Escape          = 0x0001,
    One             = 0x0002, // 1 or ! with shift
    Two             = 0x0003, // 2 or @ with shift
    Three           = 0x0004, // 3 or # with shift
    Four            = 0x0005, // 4 or $ with shift
    Five            = 0x0006, // 5 or % with shift
    Six             = 0x0007, // 6 or ^ with shift
    Seven           = 0x0008, // 7 or & with shift
    Eight           = 0x0009, // 8 or * with shift
    Nine            = 0x000A, // 9 or ( with shift
    Zero            = 0x000B, // 0 or ) with shift
    MinusUnderscore = 0x000C, // - or _ with shift
    PlusEqual       = 0x000D, // + or = with shift
    Backspace       = 0x000E, // Backspace
    Tab             = 0x000F, // Tab
    Q               = 0x0010, // q or Q with shift
    W               = 0x0011, // w or W with shift
    E               = 0x0012, // e or E with shift
    R               = 0x0013, // r or R with shift
    T               = 0x0014, // t or T with shift
    Y               = 0x0015, // y or Y with shift
    U               = 0x0016, // u or U with shift
    I               = 0x0017, // i or I with shift
    O               = 0x0018, // o or O with shift
    P               = 0x0019, // p or P with shift
    LeftBracket     = 0x001A, // [ or { with shift
    RightBracket    = 0x001B, // ] or } with shift
    Enter           = 0x001C, // Enter or Return
    LeftControl     = 0x001D, // Left Control
    A               = 0x001E, // a or A with shift
    S               = 0x001F, // s or S with shift
    D               = 0x0020, // d or D with shift
    F               = 0x0021, // f or F with shift
    G               = 0x0022, // g or G with shift
    H               = 0x0023, // h or H with shift
    J               = 0x0024, // j or J with shift
    K               = 0x0025, // k or K with shift
    L               = 0x0026, // l or L with shift
    SemicolonColon  = 0x0027, // ; or : with shift
    ApostropheQuote = 0x0028, // ' or " with shift
    BacktickTilde   = 0x0029, // ` or ~ with shift
    LeftShift       = 0x002A, // Left Shift
    BackslashPipe   = 0x002B, // \ or | with shift
    Z               = 0x002C, // z or Z with shift
    X               = 0x002D, // x or X with shift
    C               = 0x002E, // c or C with shift
    V               = 0x002F, // v or V with shift
    B               = 0x0030, // b or B with shift
    N               = 0x0031, // n or N with shift
    M               = 0x0032, // m or M with shift
    CommaLeftAngle  = 0x0033, // , or < with shift
    DotRightAngle   = 0x0034, // . or > with shift
    SlashQuestion   = 0x0035, // / or ? with shift
    RightShift      = 0x0036, // Right Shift
    KeypadAsterisk  = 0x0037, // Keypad *
    LeftAlt         = 0x0038, // Left Alt
    Space           = 0x0039,
    CapsLock        = 0x003A, // Caps Lock
    F1              = 0x003B, // F1
    F2              = 0x003C, // F2
    F3              = 0x003D, // F3
    F4              = 0x003E, // F4
    F5              = 0x003F, // F5
    F6              = 0x0040, // F6
    F7              = 0x0041, // F7
    F8              = 0x0042, // F8
    F9              = 0x0043, // F9
    F10             = 0x0044, // F10
    NumLock         = 0x0045, // Num Lock
    ScrollLock      = 0x0046, // Scroll Lock
    Numpad7         = 0x0047, // Keypad 7
    Numpad8         = 0x0048, // Keypad 8
    Numpad9         = 0x0049, // Keypad 9
    NumpadMinus     = 0x004A, // Keypad -
    Numpad4         = 0x004B, // Keypad 4
    Numpad5         = 0x004C, // Keypad 5
    Numpad6         = 0x004D, // Keypad 6
    NumpadPlus      = 0x004E, // Keypad +
    Numpad1         = 0x004F, // Keypad 1
    Numpad2         = 0x0050, // Keypad 2
    Numpad3         = 0x0051, // Keypad 3
    Numpad0         = 0x0052, // Keypad 0
    NumpadDot       = 0x0053, // Keypad .
    F11             = 0x0057, // F11
    F12             = 0x0058, // F12
    // Extended Keyboard keys E0 and then (KeyCode & 0x7F)
    PrevTrack       = 0xE010, // Previous Track
    NextTrack       = 0xE019, // Next Track
    NumpadEnter     = 0xE01C, // Keypad Enter
    RightControl    = 0xE01D, // Right Control
    Mute            = 0xE020, // Mute
    PlayPause       = 0xE022, // Play/Pause
    Stop            = 0xE024, // Stop
    VolumeDown      = 0xE02E, // Volume Down
    VolumeUp        = 0xE030, // Volume Up
    RightAlt        = 0xE038, // Right Alt
    Home            = 0xE047, // Home
    Up              = 0xE048, // Up Arrow
    PageUp          = 0xE049, // Page Up
    Left            = 0xE04B, // Left Arrow
    Right           = 0xE04D, // Right Arrow
    End             = 0xE04F, // End
    Down            = 0xE050, // Down Arrow
    PageDown        = 0xE051, // Page Down
    Insert          = 0xE052, // Insert
    Delete          = 0xE053, // Delete
    LeftGUIButton   = 0xE05B, // Left GUI (Windows) Button
    RightGUIButton  = 0xE05C, // Right GUI (Windows) Button
    Apps            = 0xE05D, // Application (Menu) Button

    // Todo Mouse buttons
}
impl From<u16> for Key {
    fn from(value: u16) -> Self {
        unsafe { core::mem::transmute(value) }
    }
}
#[derive(Copy, Clone, Debug)]
pub struct KeyboardEvent {
    pub key:        Key,
    pub released:   bool,
    pub modifiers:  u8, // bit 0 = shift, bit 1 = ctrl, bit 2 = alt
}
impl KeyboardEvent {
    pub const MODIFIER_SHIFT:       u8 = 0x1;
    pub const MODIFIER_CTRL:        u8 = 0x2;
    pub const MODIFIER_ALT:         u8 = 0x4;
    pub const MODIFIER_CAPSLOCK:    u8 = 0x8;

    pub fn to_ascii(&self) -> Option<u8> {
        let keycode = self.key as usize;
        let ascii: u8;
        if self.modifiers & Self::MODIFIER_SHIFT != 0 {
            ascii = match keycode {
                0x02..0x0E  => b"!@#$%^&*()_+"[keycode - 0x02],
                0x0E        => 0x8, // Backspace
                0x0F        => b'\t', // Tab
                0x10..0x1C  => b"QWERTYUIOP{}"[keycode - 0x10],
                0x1C        => b'\n',
                0x1E..0x29  => b"ASDFGHJKL:\""[keycode - 0x1E],
                0x29        => b'~',
                0x2B        => b'|',
                0x2C..0x36  => b"ZXCVBNM<>?"[keycode - 0x2C],
                0x39        => b' ',
                _           => 0
            };
        } else {
            ascii = match keycode {
                0x02..0x0E  => b"1234567890-="[keycode - 0x02],
                0x0E        => 0x8, // Backspace
                0x0F        => b'\t', // Tab
                0x10..0x1C  => b"qwertyuiop[]"[keycode - 0x10],
                0x1C        => b'\n',
                0x1E..0x29  => b"asdfghjkl;'"[keycode - 0x1E],
                0x29        => b'`',
                0x2B        => b'\\',
                0x2C..0x36  => b"zxcvbnm,./"[keycode - 0x2C],
                0x39        => b' ',
                _           => 0
            };
        }
        if ascii != 0 {
            Some(ascii)
        } else {
            None
        }
    }

    pub fn alt_pressed(&self) -> bool {
        self.modifiers & Self::MODIFIER_ALT != 0
    }

    pub fn ctrl_pressed(&self) -> bool {
        self.modifiers & Self::MODIFIER_CTRL != 0
    }

    pub fn shift_pressed(&self) -> bool {
        self.modifiers & Self::MODIFIER_SHIFT != 0
    }
}


pub struct MouseEvent {
    pub x:          u32,
    pub y:          u32,
    pub z:          u32,
    pub left_btn:   bool,
    pub right_btn:  bool,
    pub middle_btn: bool
}
pub struct Mouse {
    mouse_file:     File,
}
impl Mouse {
    pub const fn new() -> Self {
        Self {
            mouse_file: File::new(),
        }
    }

    pub fn start_listening(&mut self) -> Result<(), Exception> {
        if self.mouse_file.is_open() {
            // Already listening
            return Ok(());
        }
        let path = Path::from("mouse:/all");
        self.mouse_file.open(&path, File::MODE_READ | File::MODE_STREAM)?;
        Ok(())
    }

    pub fn stop_listening(&mut self) {
        self.mouse_file.close();
    }

    pub fn fetch_events(&mut self) -> Vec<MouseEvent> {
        let mut events = Vec::new();
        // Read the first byte of the keycode
        let mut buf = [0u8; 512];
        let mut len: usize = 0;
        if self.mouse_file.is_open() {
            if let Ok(l) = self.mouse_file.read(&mut buf) {
                len = l;
            } else {
                // Error reading from the mouse file
                println!("E1");
                len = 0;
            }
        }
        let Ok(str_events) = str::from_utf8(&buf[0..len]) else {
            return events;
        };
        for strev in str_events.split("\n") {
            let fields: Vec<&str> = strev.split(",").collect();
            if fields.len() != 4 {
                continue;
            }
            let Ok(x) = u32::from_str_radix(fields[0], 16) else {continue};
            let Ok(y) = u32::from_str_radix(fields[1], 16) else {continue};
            let Ok(z) = u32::from_str_radix(fields[2], 16) else {continue};
            let Ok(b) = u32::from_str_radix(fields[3], 16) else {continue};
            events.push(MouseEvent {
                x, y, z,
                left_btn: b & 0x1 > 0,
                right_btn: b & 0x2 > 0,
                middle_btn: b & 0x4 > 0 
            });
        }
        events
    }
}
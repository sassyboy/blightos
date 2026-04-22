//
// BlightOS Kernel
//
// Human Input Device Interface
//

use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicBool;
use alloc::string::String;
use alloc::format;
use crate::fs::{DirectoryEntry, FileOperation, MountPoint};
use alloc::vec::Vec;
use crate::sched::WaitChannel;
use crate::util::*;

#[cfg(target_arch = "x86_64")]
pub mod i8042;
#[cfg(target_arch = "aarch64")]
pub mod uartkbd;


//
// Mouse Interface
//
pub struct Mouse {
    cur_x:      u32,
    cur_y:      u32,
    max_x:      u32,
    max_y:      u32,
    xdiv:       i32,
    ydiv:       i32,
    btns_sts:   u8,
    initialized:bool, // Set true when the first Mouse driver registers
    events:     Vec<(u32, u32, u32, u16)>, //x, y, z, button mask
    irq_cb:     Option<fn()> // For kernel's gui only
}

static MOUSE: Spinlock<Mouse> = Spinlock::new(Mouse::new());
impl Mouse {
    pub const BTN_LEFT:     u8 = 0x1;
    pub const BTN_RIGHT:    u8 = 0x2;
    pub const BTN_MIDDLE:   u8 = 0x4;
    
    pub const fn new() -> Self {
        Self {
            cur_x:      0,
            cur_y:      0,
            max_x:      1000,
            max_y:      1000,
            xdiv:       1,  //Normal direction: Left=Lower,Right=Higher
            ydiv:       -1, //Reverse direction: Up=lower, Down=higher
            btns_sts:   0,
            initialized:false,
            events:     Vec::new(),
            irq_cb:     None
        }
    }

    pub fn register_mouse() {
        let mut mouse = MOUSE.lock();
        if !mouse.initialized {
            // Register the mount-point
            let mnt_obj = MountPoint {
                name:       String::from("mouse"),
                fops:       Self::fops_handler
            };
            if !MountPoint::mount(mnt_obj) {
                klog!("Failed to mount mouse:/");
                return;
            }
            mouse.initialized = true;
        }
    }

    pub fn reset_coordinates(cur_x: u32, cur_y: u32, max_x: u32, max_y: u32) {
        let mut mouse = MOUSE.lock();
        mouse.cur_x = cur_x;
        mouse.cur_y = cur_y;
        mouse.max_x = max_x;
        mouse.max_y = max_y;
    }

    pub fn set_irq_callback(cb: fn()) {
        let mut mouse = MOUSE.lock();
        mouse.irq_cb = Some(cb);
    }

    pub fn current_position() -> (u32, u32) {
        let mouse = MOUSE.lock();
        let x = mouse.cur_x;
        let y = mouse.cur_y;
        (x, y)
    }

    pub fn push(delta_x: i16, delta_y: i16, btn_mask: u8) {
        let mut mouse = MOUSE.lock();
        mouse.btns_sts = btn_mask;
        let new_x = mouse.cur_x as i32 + (delta_x as i32)/mouse.xdiv;
        let new_y = mouse.cur_y as i32 + (delta_y as i32)/mouse.ydiv;
        // Clip the absolute values
        if new_x >= 0 && new_x < mouse.max_x as i32 {
            mouse.cur_x = new_x as u32;
        } else if new_x < 0 {
            mouse.cur_x = 0;
        } else {
            mouse.cur_x = mouse.max_x;
        }
        if new_y >= 0 && new_y < mouse.max_y as i32{
            mouse.cur_y = new_y as u32;
        } else if new_y < 0 {
            mouse.cur_y = 0;
        } else {
            mouse.cur_y = mouse.max_y;
        }
        let event = (
            mouse.cur_x as u32,
            mouse.cur_y as u32,
            0,
            mouse.btns_sts as u16);
        mouse.events.push(event);
        if let Some(callback) = mouse.irq_cb {
            drop(mouse); // unlock!
            callback();
        }
    }

    //
    // VFS Interface
    //
    const MOUSE_ROOT_HND: usize = 0;
    // Special handle that receives all mouse events regardless of the
    // device in case we want to support multiple mouse devices in the future
    const MOUSE_ALL_HND:  usize = 1; 
    const MOUSE_BUFF_SIZE:usize = 1024;

    fn fops_handler(op: FileOperation) -> Result<usize, Error> {
        match op {
            FileOperation::Open { full_path, mode: _, dent } => {
                let mpath = MountPoint::device_relative_path(full_path);
                if mpath.eq("/") {
                    dent.name = String::from("");
                    dent.size = 0;
                    dent.flags = DirectoryEntry::DEV_RX_DIR_FLAGS;
                    return Ok(Self::MOUSE_ROOT_HND);
                } else if mpath.eq("/all") {
                    dent.name = String::from("all");
                    dent.size = Self::MOUSE_BUFF_SIZE;
                    dent.flags = DirectoryEntry::DEV_RX_FILE_FLAGS;
                    return Ok(Self::MOUSE_ALL_HND);
                } else {
                    return Err(error!(ErrorCode::InvalidPath));
                }
            },
            FileOperation::Enum { hnd, out } => {
                if hnd != Self::MOUSE_ROOT_HND {
                    // Only the root (kbd:/) provides a list
                    return Err(error!(ErrorCode::InvalidOp));
                }
                // List the keyboard devices
                out.push(
                    DirectoryEntry {
                    name: String::from("all"),
                    size: Self::MOUSE_BUFF_SIZE,
                    flags: DirectoryEntry::DEV_R_FILE_FLAGS
                });
                Ok(out.len())
            }
            FileOperation::Close { hnd: _ } => {
                return Ok(0);
            },
            FileOperation::Read { hnd, off: _, buff} => {
                if hnd != Self::MOUSE_ALL_HND {
                    return Err(error!(ErrorCode::InvalidHandle));
                }
                let mut mouse = MOUSE.lock();
                let mut bytes_written = 0;
                while bytes_written < buff.len() && !mouse.events.is_empty() {
                    let event = mouse.events.remove(0);
                    let dat = format!("{:X},{:X},{:X},{:X}\n",
                        event.0, event.1, event.2, event.3);
                    if buff.len() - bytes_written >= dat.len() {
                        for i in 0..dat.len() {
                            buff[bytes_written] =  dat.as_bytes()[i];
                            bytes_written += 1;
                        }
                    }
                }
                return Ok(bytes_written);
            },
            FileOperation::Exec { hnd: _, func: _, buff: _ } => {
                return Err(error!(ErrorCode::InvalidOp));
            },
            FileOperation::Write { hnd: _, off: _, buff: _ } => {
                return Err(error!(ErrorCode::InvalidOp));
            }
        }
    }
}

//
// Keyboard Interface
//
// Provides a unified interface for all keyboard-like input devices
// (including uart kbd).
//
// Every enumerated keyboard device must register itself with this interface
// via a call to Keyboard::register() to be accessible by the kernel and
// user-space tasks.
//
// The kernel can use the struct methods to access any of the available
// keyboard devices without needing to know the specifics of each device.
// Similarly, user-space tasks use the VFS interface (i.e., read from kbd:/).
//

pub enum KeyboardEvent {
    KeyPressed,
    KeyReleased,
    KeyPressedOrReleased
}

pub struct KeyboardListener {
    pub lhnd: usize,        // Handle to identify the listener task
    pub dhnd: usize,        // Handle to identify the keyboard device the 
                            // listener is subscribed to.
    pub events: Vec<u8>,
}
pub struct Keyboard {
    initialized:bool,
    devices:    Vec<String>, // Registered keyboards
    listeners:  Vec<KeyboardListener>,
    next_hnd:   usize,  // Listener's handle starts from KBD_LISTENER_INIT_HND.
                        // 0..0x100 is reserved for other possible nodes under
                        // kbd:/
    next_dev:   usize,  // Handler for the next registered keyboard device.
}
static KEYBOARD: Spinlock<Keyboard> = Spinlock::new(Keyboard::new());
impl Keyboard {
    pub const KEY_RELEASED: u8 = 0x80;

    pub const fn new() -> Self {
        Self {
            initialized: false,
            devices: Vec::new(),
            listeners: Vec::new(),
            next_hnd: Self::KBD_LISTENER_INIT_HND,
            next_dev: Self::KBD_ALL_HND + 1,
        }
    }

    /// Registers an enumerated keyboard device with the interface.
    ///
    /// Returns a handle to the registered device that can be used to 
    /// distinguish events from different devices.
    pub fn register_keyboard(name: &str) -> usize {
        let mut kbd = KEYBOARD.lock();
        if !kbd.initialized {
            // Register the mount-point
            let mnt_obj = MountPoint {
                name:       String::from("kbd"),
                fops:       Self::fops_handler
            };
            if !MountPoint::mount(mnt_obj) {
                klog!("Failed to mount kbd:/");
                return 0;
            }
            kbd.devices.push(String::from("all"));
            kbd.initialized = true;
        }
        
        kbd.devices.push(String::from(name));
        let dev_hnd = kbd.next_dev;
        kbd.next_dev += 1;
        return dev_hnd;
    }

    //
    // Public interface for the kernel
    //
    pub fn clear_buffer() {
        KBD_LAST_KEYCODE.store(0, Relaxed);
    }

    pub fn wait_for_event(event: KeyboardEvent) {
        match event {
            KeyboardEvent::KeyPressedOrReleased =>  {KBD_WC_ANY.wait();},
            KeyboardEvent::KeyPressed           =>  {KBD_WC_PRESS.wait();}
            KeyboardEvent::KeyReleased          =>  {KBD_WC_RELE.wait();}
        } 
    }

    pub fn pop_ascii() -> u8 {
        let kc = KBD_LAST_KEYCODE.load(Relaxed);
        KBD_LAST_KEYCODE.store(0, Relaxed);
        return Self::keycode_to_ascii(kc);
    }

    pub fn keycode_to_ascii(keycode: u8) -> u8 {
        if KBD_LSHIFT.load(Relaxed) == true ||
           KBD_RSHIFT.load(Relaxed) == true  {
            match keycode {
                0x02..0x0E  => b"!@#$%^&*()_+"[keycode as usize - 0x02],
                0x0E        => 0x8, // Backspace
                0x0F        => b' ', // Tab
                0x10..0x1C  => b"QWERTYUIOP{}"[keycode as usize - 0x10],
                0x1C        => b'\n',
                0x1E..0x29  => b"ASDFGHJKL:\""[keycode as usize - 0x1E],
                0x29        => b'~',
                0x2B        => b'|',
                0x2C..0x36  => b"ZXCVBNM<>?"[keycode as usize - 0x2C],
                0x39        => b' ',
                _           => 0
            }
        } else {
            match keycode {
                0x02..0x0E  => b"1234567890-="[keycode as usize - 0x02],
                0x0E        => 0x8, // Backspace
                0x0F        => b' ', // Tab
                0x10..0x1C  => b"qwertyuiop[]"[keycode as usize - 0x10],
                0x1C        => b'\n',
                0x1E..0x29  => b"asdfghjkl;'"[keycode as usize - 0x1E],
                0x29        => b'`',
                0x2B        => b'\\',
                0x2C..0x36  => b"zxcvbnm,./"[keycode as usize - 0x2C],
                0x39        => b' ',
                _           => 0
            }
        }
    }

    pub fn ascii_to_keycode(ascii: u8) -> u8 {
        if ascii < 128 {
            ASCII_TO_KEYCODE[ascii as usize].0
        } else {
            0
        }
    }

    //
    // Public interface to be exclusively used by specific keyboard drivers
    //
    pub fn push(kbd_hnd: usize, code: u8) {

        // Add the keycode to the event queue of all listeners
        {
            // Todo: offload if kbd can't be locked immediately
            let mut kbd = KEYBOARD.lock();
            for lst in kbd.listeners.iter_mut() {
                if lst.dhnd == Self::KBD_ALL_HND || lst.dhnd == kbd_hnd {
                    lst.events.push(code);
                }
            }
        }
        // TODO - remove this once stdin is figured out
        if code & 0x7F == KeyCode::LeftShift as u8 {
            if code & Self::KEY_RELEASED > 0 {
                KBD_LSHIFT.store(false, Relaxed);
            } else {
                KBD_LSHIFT.store(true, Relaxed);
            }
        } else if code & 0x7F == KeyCode::RightShift as u8 {
            if code & Self::KEY_RELEASED > 0 {
                KBD_RSHIFT.store(false, Relaxed);
            } else {
                KBD_RSHIFT.store(true, Relaxed);
            }
        } else {
            KBD_LAST_KEYCODE.store(code, Relaxed);
        }
        // Wake up the tasks waiting a keyboard event
        if code & Self::KEY_RELEASED > 0 {
            KBD_WC_RELE.signal_all();
        } else {
            KBD_WC_PRESS.signal_all();
        }
        KBD_WC_ANY.signal_all();
    }

    pub fn push_ascii(kbd_hnd: usize, ascii: u8) {
        let (code, shift) = if ascii < 128 {
            ASCII_TO_KEYCODE[ascii as usize]
        } else {
            (0, false)
        };
        if code == 0 {
            return; // No mapping for this ascii code
        }
        // Simulate the corresponding series of key events
        if shift {
            Self::push(kbd_hnd, KeyCode::LeftShift as u8);
        }
        Self::push(kbd_hnd, code);
        Self::push(kbd_hnd, code | Self::KEY_RELEASED);
        if shift {
            Self::push(kbd_hnd, KeyCode::LeftShift as u8 | Self::KEY_RELEASED);
        }
    }

    //
    // VFS interface for reading from kbd:/
    // Every user-space task that opens kbd:/ will get its own independent
    // event queue that gets populated by the push() function above. This allows
    // multiple tasks to read from the keyboard without interfering with each
    // other.
    //
    const KBD_ROOT_HND: usize = 0;
    // Special handle that receives all keyboard events regardless of the
    // device in case we want to support multiple keyboard devices in the future
    const KBD_ALL_HND:  usize = 1; 
    const KBD_LISTENER_INIT_HND: usize = 0x100;
    const KBD_BUFF_SIZE: usize = 128; // Max number of codes to read at a time

    fn fops_handler(op: FileOperation) -> Result<usize, Error> {
        match op {
            FileOperation::Open { full_path, mode: _, dent } => {
                let mpath = MountPoint::device_relative_path(full_path);
                if mpath.eq("/") {
                    dent.name = String::from("");
                    dent.size = 0;
                    dent.flags = DirectoryEntry::DEV_RX_DIR_FLAGS;
                    return Ok(Self::KBD_ROOT_HND);
                } else if mpath.eq("/all") {
                    dent.name = String::from("all");
                    dent.size = Self::KBD_BUFF_SIZE;
                    dent.flags = DirectoryEntry::DEV_RX_FILE_FLAGS;
                    // Insert a listener for this caller
                    let mut kbd = KEYBOARD.lock();
                    let listener_hnd = kbd.next_hnd;
                    kbd.listeners.push(KeyboardListener {
                        lhnd: listener_hnd,
                        dhnd: Self::KBD_ALL_HND,
                        events: Vec::new()
                    });
                    kbd.next_hnd += 1;
                    return Ok(listener_hnd);
                } else {
                    // Look in the registered devices list for a match
                    let mut kbd = KEYBOARD.lock();
                    let mut found_dev = false;
                    let mut dev_index = 0;
                    for (i, dev_name) in kbd.devices.iter().enumerate() {
                        if mpath.starts_with("/") && mpath.ends_with(dev_name)
                            && mpath.len() == dev_name.len() + 1 {
                            dent.name = dev_name.clone();
                            dent.size = Self::KBD_BUFF_SIZE;
                            dent.flags = DirectoryEntry::DEV_RX_FILE_FLAGS;
                            dev_index = i;
                            found_dev = true;
                            break;
                        }
                    }
                    if !found_dev {
                        return Err(error!(ErrorCode::InvalidPath));
                    }
                    // Insert a listener for this caller
                    let listener_hnd = kbd.next_hnd;
                    kbd.listeners.push(KeyboardListener {
                        lhnd: listener_hnd,
                        dhnd: dev_index + Self::KBD_ALL_HND,
                        events: Vec::new()
                    });
                    kbd.next_hnd += 1;
                    return Ok(listener_hnd);
                }
            },
            FileOperation::Enum { hnd, out } => {
                if hnd != Self::KBD_ROOT_HND {
                    // Only the root (kbd:/) provides a list
                    return Err(error!(ErrorCode::InvalidOp));
                }
                // List the keyboard devices
                let kbd = KEYBOARD.lock();
                for (_i, dev_name) in kbd.devices.iter().enumerate() {
                    out.push(
                        DirectoryEntry {
                        name: dev_name.clone(),
                        size: Self::KBD_BUFF_SIZE,
                        flags: DirectoryEntry::DEV_R_FILE_FLAGS
                    });
                }
                Ok(out.len())
            }
            FileOperation::Close { hnd } => {
                // Find the listener for this handle and remove it
                let mut kbd = KEYBOARD.lock();
                if let Some(pos) = kbd.listeners.iter().position(|l| l.lhnd == hnd) {
                    kbd.listeners.remove(pos);
                } else {
                    return Err(error!(ErrorCode::InvalidHandle));
                }
                return Ok(0);
            },
            FileOperation::Read { hnd, off: _, buff} => {
                // Find the listener for this handle
                let mut kbd = KEYBOARD.lock();
                if let Some(lst) = kbd.listeners.iter_mut()
                                                    .find(|l| l.lhnd == hnd) {
                    // Read events from the listener's event queue
                    
                    let mut bytes_written = 0;
                    while bytes_written < buff.len() && !lst.events.is_empty() {
                        buff[bytes_written] = lst.events.remove(0);
                        bytes_written += 1;
                    }
                    return Ok(bytes_written);
                } else {
                    return Err(error!(ErrorCode::InvalidHandle));
                }
            },
            FileOperation::Exec { hnd: _, func: _, buff: _ } => {
                return Err(error!(ErrorCode::InvalidOp));
            },
            FileOperation::Write { hnd: _, off: _, buff: _ } => {
                return Err(error!(ErrorCode::InvalidOp));
            }
        }
    }
}

static KBD_LSHIFT:        AtomicBool = AtomicBool::new(false);
static KBD_RSHIFT:        AtomicBool = AtomicBool::new(false);
static KBD_LAST_KEYCODE:  AtomicU8 = AtomicU8::new(0);
static KBD_WC_PRESS:      WaitChannel = WaitChannel::new();
static KBD_WC_RELE:       WaitChannel = WaitChannel::new();
static KBD_WC_ANY:        WaitChannel = WaitChannel::new();

// (Keycode & 0x7F)
#[repr(u8)] 
pub enum KeyCode {
    Escape          = 0x01,
    One             = 0x02, // 1 or ! with shift
    Two             = 0x03, // 2 or @ with shift
    Three           = 0x04, // 3 or # with shift
    Four            = 0x05, // 4 or $ with shift
    Five            = 0x06, // 5 or % with shift
    Six             = 0x07, // 6 or ^ with shift
    Seven           = 0x08, // 7 or & with shift
    Eight           = 0x09, // 8 or * with shift
    Nine            = 0x0A, // 9 or ( with shift
    Zero            = 0x0B, // 0 or ) with shift
    MinusUnderscore = 0x0C, // - or _ with shift
    PlusEqual       = 0x0D, // + or = with shift
    Backspace       = 0x0E, // Backspace
    Tab             = 0x0F, // Tab
    Q               = 0x10, // q or Q with shift
    W               = 0x11, // w or W with shift
    E               = 0x12, // e or E with shift
    R               = 0x13, // r or R with shift
    T               = 0x14, // t or T with shift
    Y               = 0x15, // y or Y with shift
    U               = 0x16, // u or U with shift
    I               = 0x17, // i or I with shift
    O               = 0x18, // o or O with shift
    P               = 0x19, // p or P with shift
    LeftBracket     = 0x1A, // [ or { with shift
    RightBracket    = 0x1B, // ] or } with shift
    Enter           = 0x1C, // Enter or Return
    LeftControl     = 0x1D, // Left Control
    A               = 0x1E, // a or A with shift
    S               = 0x1F, // s or S with shift
    D               = 0x20, // d or D with shift
    F               = 0x21, // f or F with shift
    G               = 0x22, // g or G with shift
    H               = 0x23, // h or H with shift
    J               = 0x24, // j or J with shift
    K               = 0x25, // k or K with shift
    L               = 0x26, // l or L with shift
    SemicolonColon  = 0x27, // ; or : with shift
    ApostropheQuote = 0x28, // ' or " with shift
    BacktickTilde   = 0x29, // ` or ~ with shift
    LeftShift       = 0x2A, // Left Shift
    BackslashPipe   = 0x2B, // \ or | with shift
    Z               = 0x2C, // z or Z with shift
    X               = 0x2D, // x or X with shift
    C               = 0x2E, // c or C with shift
    V               = 0x2F, // v or V with shift
    B               = 0x30, // b or B with shift
    N               = 0x31, // n or N with shift
    M               = 0x32, // m or M with shift
    CommaLeftAngle  = 0x33, // , or < with shift
    DotRightAngle   = 0x34, // . or > with shift
    SlashQuestion   = 0x35, // / or ? with shift
    RightShift      = 0x36, // Right Shift
    KeypadAsterisk  = 0x37, // Keypad *
    LeftAlt         = 0x38, // Left Alt
    Space           = 0x39,
    CapsLock        = 0x3A, // Caps Lock
    F1              = 0x3B, // F1
    F2              = 0x3C, // F2
    F3              = 0x3D, // F3
    F4              = 0x3E, // F4
    F5              = 0x3F, // F5
    F6              = 0x40, // F6
    F7              = 0x41, // F7
    F8              = 0x42, // F8
    F9              = 0x43, // F9
    F10             = 0x44, // F10
    NumLock         = 0x45, // Num Lock
    ScrollLock      = 0x46, // Scroll Lock
    Numpad7         = 0x47, // Keypad 7
    Numpad8         = 0x48, // Keypad 8
    Numpad9         = 0x49, // Keypad 9
    NumpadMinus     = 0x4A, // Keypad -
    Numpad4         = 0x4B, // Keypad 4
    Numpad5         = 0x4C, // Keypad 5
    Numpad6         = 0x4D, // Keypad 6
    NumpadPlus      = 0x4E, // Keypad +
    Numpad1         = 0x4F, // Keypad 1
    Numpad2         = 0x50, // Keypad 2
    Numpad3         = 0x51, // Keypad 3
    Numpad0         = 0x52, // Keypad 0
    NumpadDot       = 0x53, // Keypad .
    F11             = 0x57, // F11
    F12             = 0x58, // F12
    ExtendedCode    = 0x60, // 0xE0 & 0x7F = 0x60, i.e., start of extended
                            // keycodes (e.g., arrow keys) and is followed by
                            // another keycode from ExtendedKeyCode enum
}

// Extended Keycode & 0x7F
#[repr(u8)] 
pub enum ExtendedKeyCode {
    PrevTrack       = 0x10, // Previous Track
    NextTrack       = 0x19, // Next Track
    NumpadEnter     = 0x1C, // Keypad Enter
    RightControl    = 0x1D, // Right Control
    Mute            = 0x20, // Mute
    PlayPause       = 0x22, // Play/Pause
    Stop            = 0x24, // Stop
    VolumeDown      = 0x2E, // Volume Down
    VolumeUp        = 0x30, // Volume Up
    RightAlt        = 0x38, // Right Alt
    Home            = 0x47, // Home
    Up              = 0x48, // Up Arrow
    PageUp          = 0x49, // Page Up
    Left            = 0x4B, // Left Arrow
    Right           = 0x4D, // Right Arrow
    End             = 0x4F, // End
    Down            = 0x50, // Down Arrow
    PageDown        = 0x51, // Page Down
    Insert          = 0x52, // Insert
    Delete          = 0x53, // Delete
    LeftGUIButton   = 0x5B, // Left GUI (Windows) Button
    RightGUIButton  = 0x5C, // Right GUI (Windows) Button
    Apps            = 0x5D, // Application (Menu) Button
}


// (keycode & 0x7F, shift)
pub const ASCII_TO_KEYCODE: [(u8, bool); 128] = [
    // 0..31: control -> 0
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0x0F, false),  // 9  '\t' -> KEY_TAB
    (0x1C, false),  // 10 '\n' -> KEY_ENTER
    (0, false),
    (0, false),
    (0x1C, false),  // 10 '\n' -> KEY_RETURN
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    (0, false),
    // 32..47
    (0x39, false),  // 32 ' ' -> KEY_SPACE
    (0x02, true),   // 33 '!' -> '1'  + shift
    (0x28, true),   // 34 '"' -> '\'' + shift
    (0x04, true),   // 35 '#' -> '3' + shift
    (0x05, true),   // 36 '$' -> '4' + shift
    (0x06, true),   // 37 '%' -> '5' + shift
    (0x08, true),   // 38 '&' -> '7' + shift
    (0x28, false),  // 39 '\'' -> KEY_APOSTROPHE
    (0x0A, true),   // 40 '(' -> '9' + shift
    (0x0B, true),   // 41 ')' -> '0' + shift
    (0x09, true),   // 42 '*' -> '8' + shift
    (0x0D, true),   // 43 '+' -> '=' + shift
    (0x33, false),  // 44 ',' -> KEY_COMMA
    (0x0C, false),  // 45 '-' -> KEY_MINUS
    (0x34, false),  // 46 '.' -> KEY_DOT
    (0x35, false),  // 47 '/' -> KEY_SLASH
    // 48..57 digits
    (0x0B, false),  // 48 '0' -> KEY_0
    (0x02, false),  // 49 '1' -> KEY_1
    (0x03, false),  // 50 '2' -> KEY_2
    (0x04, false),  // 51 '3' -> KEY_3
    (0x05, false),  // 52 '4' -> KEY_4
    (0x06, false),  // 53 '5' -> KEY_5
    (0x07, false),  // 54 '6' -> KEY_6
    (0x08, false),  // 55 '7' -> KEY_7
    (0x09, false),  // 56 '8' -> KEY_8
    (0x0A, false),  // 57 '9' -> KEY_9
    // 58..64
    (0x27, true),   // 58 ':' -> ';' + shift
    (0x27, false),  // 59 ';' -> KEY_SEMICOLON
    (0x33, true),   // 60 '<' -> ',' + shift
    (0x0D, false),  // 61 '=' -> KEY_EQUAL (46)
    (0x34, true),   // 62 '>' -> '.' + shift
    (0x35, true),   // 63 '?' -> '/' + shift
    (0x03, true),   // 64 '@' -> '2' + shift
    // 65..90 uppercase letters -> map to same keycodes as lowercase (shift handled elsewhere)
    (0x1E, true),   // 65 'A' -> KEY_A + shift
    (0x30, true),   // 66 'B' -> KEY_B + shift
    (0x2E, true),   // 67 'C' -> KEY_C + shift
    (0x20, true),   // 68 'D' -> KEY_D + shift
    (0x12, true),   // 69 'E' -> KEY_E + shift
    (0x21, true),   // 70 'F' -> KEY_F + shift
    (0x22, true),   // 71 'G' -> KEY_G + shift
    (0x23, true),   // 72 'H' -> KEY_H + shift
    (0x17, true),   // 73 'I' -> KEY_I + shift
    (0x24, true),   // 74 'J' -> KEY_J + shift
    (0x25, true),   // 75 'K' -> KEY_K + shift
    (0x26, true),   // 76 'L' -> KEY_L + shift
    (0x32, true),   // 77 'M' -> KEY_M + shift
    (0x31, true),   // 78 'N' -> KEY_N + shift
    (0x18, true),   // 79 'O' -> KEY_O + shift
    (0x19, true),   // 80 'P' -> KEY_P + shift
    (0x10, true),   // 81 'Q' -> KEY_Q + shift
    (0x13, true),   // 82 'R' -> KEY_R + shift
    (0x1F, true),   // 83 'S' -> KEY_S + shift
    (0x14, true),   // 84 'T' -> KEY_T + shift
    (0x16, true),   // 85 'U' -> KEY_U + shift
    (0x2F, true),   // 86 'V' -> KEY_V + shift
    (0x11, true),   // 87 'W' -> KEY_W + shift
    (0x2D, true),   // 88 'X' -> KEY_X + shift
    (0x15, true),   // 89 'Y' -> KEY_Y + shift
    (0x2C, true),   // 90 'Z' -> KEY_Z + shift
    // 91..96 punctuation
    (0x1A, false),  // 91 '[' -> KEY_LEFTBRACE
    (0x2B, false),  // 92 '\' -> KEY_BACKSLASH
    (0x1B, false),  // 93 ']' -> KEY_RIGHTBRACE
    (0x07, true),   // 94 '^' -> '6' + shift
    (0x0C, true),   // 95 '_' -> '-' + shift
    (0x29, false),  // 96 '`' -> KEY_TICK
    // 97..122 lowercase letters a..z -> 4..29
    (0x1E, false),  // 97 'a' -> KEY_A
    (0x30, false),  // 66 'b' -> KEY_B
    (0x2E, false),  // 67 'c' -> KEY_C
    (0x20, false),  // 68 'd' -> KEY_D
    (0x12, false),  // 69 'e' -> KEY_E
    (0x21, false),  // 70 'f' -> KEY_F
    (0x22, false),  // 71 'g' -> KEY_G
    (0x23, false),  // 72 'h' -> KEY_H
    (0x17, false),  // 73 'i' -> KEY_I
    (0x24, false),  // 74 'j' -> KEY_J
    (0x25, false),  // 75 'k' -> KEY_K
    (0x26, false),  // 76 'l' -> KEY_L
    (0x32, false),  // 77 'm' -> KEY_M
    (0x31, false),  // 78 'n' -> KEY_N
    (0x18, false),  // 79 'o' -> KEY_O
    (0x19, false),  // 80 'p' -> KEY_P
    (0x10, false),  // 81 'q' -> KEY_Q
    (0x13, false),  // 82 'r' -> KEY_R
    (0x1F, false),  // 83 's' -> KEY_S
    (0x14, false),  // 84 't' -> KEY_T
    (0x16, false),  // 85 'u' -> KEY_U
    (0x2F, false),  // 86 'v' -> KEY_V
    (0x11, false),  // 87 'w' -> KEY_W
    (0x2D, false),  // 88 'x' -> KEY_X
    (0x15, false),  // 89 'y' -> KEY_Y
    (0x2C, false),  // 90 'z' -> KEY_Z
    // 123..126 braces/tilde
    (0x1A, true),   // 123 '{' -> '[' + shift
    (0x2B, true),   // 124 '|' -> '\' + shift
    (0x1B, true),   // 125 '}' -> ']' + shift
    (0x29, true),   // 126 '~' -> '`' + shift
    // 127 DEL
    (0x0E, false),
];
//
// BlightOS Kernel
//
// Human Input Device Interface
//

use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicBool;
use crate::sched::WaitChannel;

#[cfg(target_arch = "x86_64")]
pub mod i8046;
#[cfg(target_arch = "aarch64")]
pub mod uartkbd;

pub enum KeyboardEvent {
    KeyPressed,
    KeyReleased,
    KeyPressedOrReleased
}

#[repr(u8)] 
pub enum KeyCode {
    LeftShift               = 0x2A,
    RightShift              = 0x36,
    Backspace               = 0x0E,
    Tab                     = 0x0F,
}

pub struct Keyboard {
}

impl Keyboard {
    //
    // Public interface
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
    
    pub fn pop_keycode() -> u8 {
        let ret = KBD_LAST_KEYCODE.load(Relaxed);
        KBD_LAST_KEYCODE.store(0, Relaxed);
        return ret;
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
    pub fn push(event: KeyboardEvent, code: u8) {
        match event {
            KeyboardEvent::KeyPressed             => {
                if code & 0x7F == KeyCode::LeftShift as u8 {
                    KBD_LSHIFT.store(true, Relaxed);
                } else if code & 0x7F == KeyCode::RightShift as u8 {
                    KBD_RSHIFT.store(true, Relaxed);
                }
                KBD_LAST_KEYCODE.store(code, Relaxed);
                KBD_WC_PRESS.signal_all();
            },
            KeyboardEvent::KeyReleased            => {
                if code & 0x7F == KeyCode::LeftShift as u8 {
                    KBD_LSHIFT.store(false, Relaxed);
                } else if code & 0x7F == KeyCode::RightShift as u8 {
                    KBD_RSHIFT.store(false, Relaxed);
                }
                // Don't record the key upon release
                KBD_WC_RELE.signal_all();
            }
            KeyboardEvent::KeyPressedOrReleased   => {
                // Some drivers (uart) can't distinguish between press/release
                KBD_LAST_KEYCODE.store(code, Relaxed);
                KBD_WC_ANY.signal_all();
                KBD_WC_RELE.signal_all();
                KBD_WC_PRESS.signal_all();
            }
        }
    }

    pub fn push_ascii(ascii: u8) {
        let (code, shift) = if ascii < 128 {
            ASCII_TO_KEYCODE[ascii as usize]
        } else {
            (0, false)
        };
        if code == 0 {
            return; // No mapping for this ascii code
        }
        if shift {
            Self::push(KeyboardEvent::KeyPressed, KeyCode::LeftShift as u8);
        } else {
            Self::push(KeyboardEvent::KeyReleased, KeyCode::LeftShift as u8);
        }
        Self::push(KeyboardEvent::KeyPressed, code);
    }

}

static KBD_LSHIFT:        AtomicBool = AtomicBool::new(false);
static KBD_RSHIFT:        AtomicBool = AtomicBool::new(false);
static KBD_LAST_KEYCODE:  AtomicU8 = AtomicU8::new(0);
static KBD_WC_PRESS:      WaitChannel = WaitChannel::new();
static KBD_WC_RELE:       WaitChannel = WaitChannel::new();
static KBD_WC_ANY:        WaitChannel = WaitChannel::new();

// (keycoard, shift)
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
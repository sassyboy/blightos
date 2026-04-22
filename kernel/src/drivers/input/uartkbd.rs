//
// BlightOS Kernel
//
// Virtual Keyboard using UART for platforms that don't have a keyboard
//

use alloc::string::String;
use core::time::Duration;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicUsize;
use crate::drivers::input::{Keyboard, KeyCode, ExtendedKeyCode};
use crate::sched::Task;
use crate::arch::bcm_peripherals::{pl011uart_getc};

pub struct UARTKeyboard {
    
}
static KBD_HND: AtomicUsize = AtomicUsize::new(0);

impl UARTKeyboard {
    pub const fn new() -> Self {
        Self {}
    }

    pub fn enumerate() -> usize {
        // Register with the Keyboard interface
        let hnd = Keyboard::register_keyboard("UARTKBD");
        KBD_HND.store(hnd, Relaxed);
        1
    }

    pub fn release( _device: usize) {
        
    }

    pub fn post_enum() {
        // Spawn a worker that checks the uart input buffer
        Task::spawn_named(Self::worker, 0, String::from("UARTKBD-WORKER"));
    }


    fn send_keycode(code: u8, released: bool) {
        let hnd = KBD_HND.load(Relaxed);
        if released  {
            Keyboard::push(hnd, code | 0x80);
        } else {
            Keyboard::push(hnd, code);
        }
    }

    fn send_key(code: KeyCode) {
        let c = code as u8;
        Self::send_keycode(c, false);
        Self::send_keycode(c, true);
    }

    fn send_ext_key(code: ExtendedKeyCode) {
        let e = KeyCode::ExtendedCode as u8;
        let c = code as u8;
        // Key pressed
        Self::send_keycode(e, true);
        Self::send_keycode(c, false);
        // Key released
        Self::send_keycode(e, true);
        Self::send_keycode(c, true);
    }

    fn worker(_arg: usize) {
        let hnd = KBD_HND.load(Relaxed);
        loop {
            if let Some(ch) = pl011uart_getc() {
                if ch == 0x1B {
                    // Terminal emulated sequence for sepcial keys
                    let Some(ch2) = pl011uart_getc() else {
                        // Just send escape and move on
                        Self::send_key(KeyCode::Escape);
                        continue;
                    };
                    let Some(ch3) = pl011uart_getc() else {
                        continue;
                    };
                    if ch2 == b'[' {
                        match ch3 {
                            b'A' => Self::send_ext_key(ExtendedKeyCode::Up),
                            b'B' => Self::send_ext_key(ExtendedKeyCode::Down),
                            b'C' => Self::send_ext_key(ExtendedKeyCode::Right),
                            b'D' => Self::send_ext_key(ExtendedKeyCode::Left),
                            _ => {}
                        }
                    } else if ch2 == b'O' {
                        match ch3 {
                            b'P' => Self::send_key(KeyCode::F1),
                            b'Q' => Self::send_key(KeyCode::F2),
                            b'R' => Self::send_key(KeyCode::F3),
                            b'S' => Self::send_key(KeyCode::F4),
                            _ => {}
                        }
                    }
                } else {
                    Keyboard::push_ascii(hnd, ch);
                }
            }
            Task::sleep(Duration::from_millis(10));
        }
    }
}

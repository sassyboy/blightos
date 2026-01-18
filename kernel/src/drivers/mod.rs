//
// BlightOS Kernel
//
// Device Driver Interface
//
//

use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicBool;

use alloc::{vec, vec::*};
use crate::arch;

pub struct DriverInfo {
    pub name:       &'static str,
    // Detect and allocate resources for comaptible device and return the number
    // of devices. Called by CPU0 without IRQs enabled
    pub enumerate:  fn() -> usize,

    // Release the device (and corresponding resources) corresponding to the
    // specified index.
    pub release:    fn(usize),

}

pub fn get_builtin_drivers() -> Vec<DriverInfo> {
    vec![
        DriverInfo {
            name: "i8046 PS/2 Controller",
            enumerate: I8046::enumerate,
            release: I8046::release
        },

        #[cfg(target_arch = "arm")]
        DriverInfo {
            name: "ARM-UART",
            enumerate: I8046::enumerate,
            release: I8046::release
        }
    ]
}

// Todo - need a standard device struct + methods to register/discover available
// devices with callbacks to do open/devcall/close for the userspace<->driver
// interactions.


pub struct I8046 {
    
}

static I8046_LSHIFT: AtomicBool = AtomicBool::new(false);
static I8046_RSHIFT: AtomicBool = AtomicBool::new(false);
static I8046_LAST_ASCII: AtomicU8 = AtomicU8::new(0);

impl I8046 {
    
    pub const fn new() -> Self {
        Self {}
    }

    fn enumerate() -> usize {
        arch::isr_register(1, Self::kdb_irq);
        // arch::cpu_unmask_irq(1);
        1
    }

    fn release( _device: usize) {
        
    }

    pub fn read_key_ascii() -> u8 {
        let ret = I8046_LAST_ASCII.load(Relaxed);
        I8046_LAST_ASCII.store(0, Relaxed);
        return ret;
    }

    const KEY_RELEASED:     u8 = 0x80;
    const LSHIFT_PRESSED:   u8 = 0x2A;
    const RSHIFT_PRESSED:   u8 = 0x36;


    fn keyboard_to_ascii(key: u8) -> u8 {
        if I8046_LSHIFT.load(Relaxed) == true ||
           I8046_RSHIFT.load(Relaxed) == true  {
            match key {
                0x02..0x0E  => b"!@#$%^&*()_+"[key as usize - 0x02],
                0x0E        => b' ', // Backspace
                0x0F        => b' ', // Tab
                0x10..0x1C  => b"QWERTYUIOP{}"[key as usize - 0x10],
                0x1C        => b'\n',
                0x1E..0x29  => b"ASDFGHJKL:\""[key as usize - 0x1E],
                0x29        => b'~',
                0x2B        => b'|',
                0x2C..0x36  => b"ZXCVBNM<>?"[key as usize - 0x2C],
                0x39        => b' ',
                _           => 0
            }
        } else {
            match key {
                0x02..0x0E  => b"1234567890-="[key as usize - 0x02],
                0x0E        => b' ', // Backspace
                0x0F        => b' ', // Tab
                0x10..0x1C  => b"qwertyuiop[]"[key as usize - 0x10],
                0x1C        => b'\n',
                0x1E..0x29  => b"asdfghjkl;'"[key as usize - 0x1E],
                0x29        => b'`',
                0x2B        => b'\\',
                0x2C..0x36  => b"zxcvbnm,./"[key as usize - 0x2C],
                0x39        => b' ',
                _           => 0
            }
        }

    }

    fn kdb_irq(_irq: u16) {
        let keycode = arch::x86_ioport_read(0x60);
        // klog!("<{:X}>", keycode);
        if keycode & Self::KEY_RELEASED > 0
        {
            match keycode & 0x7F {
                Self::LSHIFT_PRESSED    => {I8046_LSHIFT.store(false, Relaxed);},
                Self::RSHIFT_PRESSED    => {I8046_RSHIFT.store(false, Relaxed);},
                _                       => ()
            }
        } else {
            // Key press
            match keycode & 0x7F {
                Self::LSHIFT_PRESSED    => {I8046_LSHIFT.store(true, Relaxed);},
                Self::RSHIFT_PRESSED    => {I8046_RSHIFT.store(true, Relaxed);},
                _                       => {
                    I8046_LAST_ASCII.store(
                        Self::keyboard_to_ascii(keycode), Relaxed);
                }
            }
            
        }
    }
}
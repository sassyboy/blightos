//
// BlightOS Kernel
//
// Keyboard Driver(s)
//

use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicBool;
use crate::sched::WaitChannel;
use crate::arch;

pub enum KeyboardEvent {
    KeyPressed,
    KeyReleased,
    KeyPressedOrReleased
}


pub struct I8046Keyboard {
    
}

static I8046_LSHIFT:        AtomicBool = AtomicBool::new(false);
static I8046_RSHIFT:        AtomicBool = AtomicBool::new(false);
static I8046_LAST_ASCII:    AtomicU8 = AtomicU8::new(0);
static I8046_WC_PRESS:      WaitChannel = WaitChannel::new();
static I8046_WC_RELE:       WaitChannel = WaitChannel::new();
static I8046_WC_ANY:        WaitChannel = WaitChannel::new();

impl I8046Keyboard {
    
    pub const fn new() -> Self {
        Self {}
    }

    pub fn enumerate() -> usize {
        // Not usually present in ACPI - should map the kdb irq manually
        crate::arch::irq_reroute(1, 1, true);
        arch::isr_register(1, Self::kdb_irq);
        arch::x86_ioport_read::<u8>(0x60); // Clear the buffer
        // arch::cpu_unmask_irq(1);
        1
    }

    pub fn release( _device: usize) {
        
    }

    pub fn read_key_ascii() -> u8 {
        let ret = I8046_LAST_ASCII.load(Relaxed);
        I8046_LAST_ASCII.store(0, Relaxed);
        return ret;
    }

    pub fn wait_for_event(event: KeyboardEvent) {
        match event {
            KeyboardEvent::KeyPressedOrReleased =>  {I8046_WC_ANY.wait();},
            KeyboardEvent::KeyPressed           =>  {I8046_WC_PRESS.wait();}
            KeyboardEvent::KeyReleased          =>  {I8046_WC_RELE.wait();}
        }        
    }

    pub fn clear_buffer() {
        I8046_LAST_ASCII.store(0, Relaxed);
    }

    const KEY_RELEASED:     u8 = 0x80;
    const LSHIFT_PRESSED:   u8 = 0x2A;
    const RSHIFT_PRESSED:   u8 = 0x36;

    fn keyboard_to_ascii(key: u8) -> u8 {
        if I8046_LSHIFT.load(Relaxed) == true ||
           I8046_RSHIFT.load(Relaxed) == true  {
            match key {
                0x02..0x0E  => b"!@#$%^&*()_+"[key as usize - 0x02],
                0x0E        => 0x8, // Backspace
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
                0x0E        => 0x8, // Backspace
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
        if keycode & Self::KEY_RELEASED > 0
        {
            match keycode & 0x7F {
                Self::LSHIFT_PRESSED    => {I8046_LSHIFT.store(false, Relaxed);},
                Self::RSHIFT_PRESSED    => {I8046_RSHIFT.store(false, Relaxed);},
                _                       => ()
            }
            I8046_WC_RELE.signal_all();
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
            I8046_WC_PRESS.signal_all();
        }
        I8046_WC_ANY.signal_all();
        
    }
}
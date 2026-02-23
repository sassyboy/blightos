//
// BlightOS Kernel
//
// Intel PS/2 Keyboard Driver
//

use crate::drivers::input::{Keyboard, KeyboardEvent};


pub struct I8046Keyboard {
    
}

impl I8046Keyboard {
    
    pub const fn new() -> Self {
        Self {}
    }

    pub fn enumerate() -> usize {
        // Not usually present in ACPI - should map the kdb irq manually
        crate::arch::irq_reroute(1, 1, true);
        crate::arch::isr_register(1, Self::kdb_irq);
        crate::arch::x86_ioport_read::<u8>(0x60); // Clear the buffer
        // arch::cpu_unmask_irq(1);
        1
    }

    pub fn release( _device: usize) {
        
    }

    const KEY_RELEASED:     u8 = 0x80;

    fn kdb_irq(_irq: u16) {
        let keycode = crate::arch::x86_ioport_read(0x60);
        if keycode & Self::KEY_RELEASED > 0
        {
            Keyboard::push(KeyboardEvent::KeyReleased, keycode);
        } else {
            Keyboard::push(KeyboardEvent::KeyPressed, keycode);
        }
    }
}
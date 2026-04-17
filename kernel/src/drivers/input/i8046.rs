//
// BlightOS Kernel
//
// Intel PS/2 Keyboard Driver
//

use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicUsize;
use crate::drivers::input::Keyboard;

static KBD_HND: AtomicUsize = AtomicUsize::new(0);
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
        // Register with the Keyboard interface
        let hnd = Keyboard::register_keyboard("PS2KBD");
        KBD_HND.store(hnd, Relaxed);
        // arch::cpu_unmask_irq(1);
        1
    }

    pub fn release( _device: usize) {
        
    }

    fn kdb_irq(_irq: u16) {
        let keycode = crate::arch::x86_ioport_read(0x60);
        Keyboard::push(KBD_HND.load(Relaxed), keycode);
    }
}
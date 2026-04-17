//
// BlightOS Kernel
//
// Virtual Keyboard using UART for platforms that don't have a keyboard
//

use alloc::string::String;
use core::time::Duration;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicUsize;
use crate::drivers::input::Keyboard;
use crate::sched::Task;

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

    fn worker(_arg: usize) {
        loop {
            if let Some(ch) = crate::arch::bcm_peripherals::pl011uart_getc() {
                Keyboard::push_ascii(KBD_HND.load(Relaxed), ch);
            }
            Task::sleep(Duration::from_millis(10));
        }
    }
}

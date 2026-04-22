//
// BlightOS Kernel
//
// Intel PS/2 Mouse and Keyboard Driver
//

use core::hint::spin_loop;
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::AtomicUsize;
use crate::arch::{x86_ioport_read, x86_ioport_write};
use crate::drivers::input::{Keyboard, Mouse};

// use crate::util::*;
// macro_rules! dbg {
//     ($($arg:tt)*) => {
//         let mut debug_console = DebugOut;
//         let _ = write!(&mut debug_console, "[MOUSE] ");
//         let _ = write!(&mut debug_console, $($arg)*);
//     };
// }


static KBD_HND: AtomicUsize = AtomicUsize::new(0);
pub struct I8042 {
    
}

impl I8042 {
    // Note: Port 1 is Keyboard, and Port 2 is Mouse
    const IOPORT_DATA:              u16 = 0x60; // Read/Write
    const IOPORT_STATUS:            u16 = 0x64; // Read
    const IOPORT_CMD:               u16 = 0x64; // Write

    //
    // Controller commands (sent to IOPORT_CMD, response in IOPORT_DATA)
    //
    const CMD_READ_CONFIG:          u8 = 0x20;
    const CMD_WRITE_CONFIG:         u8 = 0x60;
    const CMD_PORT2_ENABLE:         u8 = 0xA8;
    // Next write to port will be written to the input buffer of port 2
    const CMD_PORT2_WRITE_INPUT:    u8 = 0xD4;

    const MOUSE_CMD_ENABLE_REPORT:  u8 = 0xF4;
    const MOUSE_CMD_1X1_SCALING:    u8 = 0xE6;
    const MOUSE_CMD_SET_RESOLUTION: u8 = 0xE8;
    const MOUSE_RESP_ACK:           u8 = 0xFA;  

    pub const fn new() -> Self {
        Self {}
    }

    pub fn enumerate() -> usize {
        // 1) Set up the keyboard
        // Not usually present in ACPI - should map the kdb irq manually
        crate::arch::irq_reroute(1, 1, true);
        crate::arch::isr_register(1, Self::kdb_irq);
        x86_ioport_read::<u8>(Self::IOPORT_DATA); // Clear the buf
        // Register with the Keyboard interface
        let hnd = Keyboard::register_keyboard("PS2KBD");
        KBD_HND.store(hnd, Relaxed);
        // arch::cpu_unmask_irq(1);

        // 2) Set up the mouse
        x86_ioport_read::<u8>(Self::IOPORT_DATA); // Clear the buf
        // CLK enable
        Self::send_cmd(Self::CMD_PORT2_ENABLE, None, false); 
        // Enable IRQ Generation
        let cfg = Self::send_cmd(Self::CMD_READ_CONFIG, None, true);
        let _ = Self::send_cmd(Self::CMD_WRITE_CONFIG, Some(cfg | 0x2), false);
        // Set 1:1 scaling
        let _ = Self::send_mouse_cmd(Self::MOUSE_CMD_1X1_SCALING);
        // Set resolution
        let _ = Self::send_mouse_cmd(Self::MOUSE_CMD_SET_RESOLUTION);
        let _ = Self::send_mouse_cmd(0); // 0=1 count/mm, 3=8 count/mm
        // Send a command to the mouse to report its status via IRQ
        let resp = Self::send_mouse_cmd(Self::MOUSE_CMD_ENABLE_REPORT);
        if resp == Self::MOUSE_RESP_ACK {
            // Mouse enabled - register the IRQ handler
            crate::arch::irq_reroute(12, 12, true);
            crate::arch::isr_register(12, Self::mouse_irq);
            // Register it
            Mouse::register_mouse();
        }
        1
    }

    /// Sends a command to the PS/2 Controller and returns the response if any
    /// arg: input argument/data for the command
    fn send_cmd(cmd: u8, arg: Option<u8>, expect_response: bool) -> u8 {
        x86_ioport_write(Self::IOPORT_CMD, cmd);
        if let Some(dat) = arg {
            x86_ioport_write(Self::IOPORT_DATA, dat);
        }
        if expect_response {
            while x86_ioport_read::<u8>(Self::IOPORT_STATUS) & 1 == 0 {
                spin_loop();
            }
            return x86_ioport_read(Self::IOPORT_DATA);
        }
        0
    }

    fn send_mouse_cmd(cmd: u8) -> u8 {
        x86_ioport_write(Self::IOPORT_CMD, Self::CMD_PORT2_WRITE_INPUT);
        x86_ioport_write(Self::IOPORT_DATA, cmd);
        while x86_ioport_read::<u8>(Self::IOPORT_STATUS) & 1 == 0 {
            spin_loop();
        }
        return x86_ioport_read(Self::IOPORT_DATA);
    }

    pub fn release( _device: usize) {
        
    }

    fn kdb_irq(_irq: u16) {
        let keycode = crate::arch::x86_ioport_read(Self::IOPORT_DATA);
        Keyboard::push(KBD_HND.load(Relaxed), keycode);
    }

    fn mouse_irq(_irq: u16) {
        // Stat bits:
        // 7: Y-Overflow, 6: X-Overflow, 5: Y-Sign, 4: X-Sign, 3: Always Set
        // 2: Middle button, 1: Right Button, 0: Left Button
        let stat:   u8;
        let movx:   u8;
        let movy:   u8;
        // let movz:   u8;
        stat = x86_ioport_read::<u8>(Self::IOPORT_DATA);
        movx = x86_ioport_read::<u8>(Self::IOPORT_DATA);
        movy = x86_ioport_read::<u8>(Self::IOPORT_DATA);
        if stat & 0xC0 > 0 {
            // Overflow => not a valid input
            return;
        }
        // movz = x86_ioport_read::<u8>(Self::IOPORT_DATA);
        let dx: i16;
        let dy: i16;
        dx = if stat & 0x10 > 0 {movx.cast_signed() as i16} else {movx as i16};
        dy = if stat & 0x20 > 0 {movy.cast_signed() as i16} else {movy as i16};
        Mouse::push(dx, dy, stat & 0x7);
        // dbg!("{:X},{:X},{:X} - btns: {:X}, dx:{}, dy:{}\n",
        //     stat, movx, movy, stat & 0x7, dx, dy);
    }
}
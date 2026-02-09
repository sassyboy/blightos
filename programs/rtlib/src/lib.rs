#![no_std]

pub mod syscall;
pub mod stdio;
pub mod fileio;
pub mod task;

use crate::stdio::*;
use crate::syscall::*;

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        let mut stdout = ConsoleOut{};
        let _ = write!(&mut stdout, $($arg)*);
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("User Program Aborting!");

    let (fname, ln) = match  info.location() {
        Some(loc) => (loc.file(), loc.line()),
        None => ("Unknown", 0)
    };
    println!("  {}:{} - {}", fname, ln, info.message());
    exit(1);
    loop {}
}


pub fn exit(status: usize) {
    syscall(Syscall::TaskControl {
        opcode: TaskControlOpCode::Exit as usize,
        args:   status,
        ret_code: 0
    });
}

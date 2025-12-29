//
// BlightOS Kernel
//
// Utility Module
//   Use this in the absence of the std library.
// 

use crate::arch::kearly_console;
pub use core::fmt::Write;

pub struct ConsoleOut;
impl Write for ConsoleOut {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        kearly_console::print_str(_s.as_bytes());
        Ok(())
    }
}

macro_rules! klog {
    ($($arg:tt)*) => {
        let mut kern_console = ConsoleOut{};
        let _ = write!(&mut kern_console, $($arg)*);
    };
}
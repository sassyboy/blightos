//
// Standard Console Input/Output
//
pub use core::fmt::Write;
use crate::*;
use crate::syscall::*;

pub struct ConsoleOut;
impl Write for ConsoleOut {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        stdio_write(_s.as_bytes());
        Ok(())
    }
}

pub fn read_line(buf: &mut [u8]) -> usize {
    let mut i = 0;
    let mut charbuf : u8;
    while i < buf.len() {
        charbuf = stdio_read_byte();
        // Don't include the new line
        if charbuf == b'\n' {
            break;
        }
        // Skip non-printables
        if charbuf != 0 {
            // A new character arrived from the keyboard
            if charbuf == 0x8 {
                // Go back one space in the buffer.
                // printing 0x8 (backspace) to stdout will take care of the display
                if i > 0 {
                    i -= 1;
                    print!("{}",charbuf as char);
                }
            } else {
                buf[i] = charbuf;
                print!("{}",charbuf as char);
                i += 1;
            }
        }
    }
    i
}

pub fn stdio_write(msg: &[u8]) {
    syscall(Syscall::Write {
            fd: SyscallRsvdFDs::StandardIO as usize,
            buf_ptr: msg.as_ptr() as usize,
            buf_len: msg.len(),
            ret_ptr: 0
    });
}

pub fn stdio_read_byte() -> u8 {
    let outchar: u8 = 0;
    syscall(Syscall::Read {
            fd: SyscallRsvdFDs::StandardIO as usize,
            buf_ptr: &outchar as *const u8 as usize,
            buf_len: 1,
            ret_ptr: 0
    });
    outchar
}


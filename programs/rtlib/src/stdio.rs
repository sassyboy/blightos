//
// Standard Console Input/Output
//
pub use core::fmt::Write;
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
    let mut args = VfsReadWriteArgs {
        fd: SyscallRsvdFDs::StandardIO as usize,
        offset: 0,
        buf_ptr: msg.as_ptr() as usize,
        buf_len: msg.len(),
        bytes: 0
    };
    let mut ret_val: usize = 0;
    syscall(Syscall::Write {
            args_ptr: &mut args as *mut VfsReadWriteArgs as usize,
            args_len: core::mem::size_of::<VfsReadWriteArgs>(),
            arg3: 0,
            ret_ptr: &mut ret_val as *mut usize as usize
    });
}

pub fn stdio_read_byte() -> u8 {
    let mut outchar: u8 = 0;
    let mut args = VfsReadWriteArgs {
        fd: SyscallRsvdFDs::StandardIO as usize,
        offset: 0,
        buf_ptr: &mut outchar as *mut u8 as usize,
        buf_len: 1,
        bytes: 0
    };
    let mut ret_val: usize = 0;
    syscall(Syscall::Read {
            args_ptr: &mut args as *mut VfsReadWriteArgs as usize,
            args_len: core::mem::size_of::<VfsReadWriteArgs>(),
            arg3: 0,
            ret_ptr: &mut ret_val as *mut usize as usize
    });
    outchar
}

pub fn stdio_clear_screen() {
    let mut func_args: [usize; 1] = [0; 1];
    let mut func_ret: usize = 0;
    let mut args = VfsExecArgs {
        fd: SyscallRsvdFDs::StandardIO as usize,
        func_code: 1, // Clear screen command
        args_ptr: func_args.as_mut_ptr() as usize,
        args_len: core::mem::size_of_val(&func_args),
        ret_val: &mut func_ret as *mut usize as usize
    };
    let mut ret_val: usize = 0;
    syscall(Syscall::Exec {
        args_ptr: &mut args as *mut VfsExecArgs as usize,
        args_len: core::mem::size_of::<VfsExecArgs>(),
        arg3: 0,
        ret_ptr: &mut ret_val as *mut usize as usize
    });
}
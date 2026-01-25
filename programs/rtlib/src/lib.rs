#![no_std]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    stdout_write(b"User Program Aborting!\n");
    exit(1);
    loop {}
}


/*******************************************************************************
 * Standard I/O
 */
pub use core::fmt::Write;

pub struct ConsoleOut;
impl Write for ConsoleOut {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        stdout_write(_s.as_bytes());
        Ok(())
    }
}

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

pub fn read_line(buf: &mut [u8]) -> usize {
    let mut i = 0;
    let charbuf : &mut [u8; 1] = &mut [0];
    while i < buf.len() {
        syscall(Syscall::Read {
            fd: 0,
            buf_ptr: charbuf as *mut u8 as usize,
            buf_len: 1,
            ret_ptr: 0
        });
        // Don't include the new line
        if charbuf[0] == b'\n' {
            break;
        }
        // Skip non-printables
        if charbuf[0] != 0 {
            // A new character arrived from the keyboard
            buf[i] = charbuf[0];
            stdout_write(charbuf);
            i += 1;
        }
    }
    i
}

/*******************************************************************************
 * BlightOS System Call Inteface
 */
use core::arch::asm;

#[repr(usize)]
#[derive(PartialEq, PartialOrd)]
pub enum SyscallOpCode {
    Exit            = 0,
    Open            = 1,
    Read            = 2,
    Write           = 3,
    Exec            = 4,
    Close           = 5,
    Max             = 6
}
pub enum Syscall {
    Exit{status: usize},
    Open{path_ptr: usize, mode: usize, ret_ptr: usize},
    Read{fd: usize, buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Write{fd: usize, buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Exec{fd: usize, cmd_buf_ptr: usize, buf_len: usize, ret_ptr: usize},
    Close{fd: usize},
}

pub fn stdout_write(msg: &[u8]) {
    syscall(Syscall::Write {
            fd: 0,
            buf_ptr: msg.as_ptr() as usize,
            buf_len: msg.len(),
            ret_ptr: 0
    });
}

pub fn exit(ret_code: usize) {
    syscall(Syscall::Exit {
        status: ret_code
    });
}


fn syscall(params: Syscall) {
    match params {
        Syscall::Exit { status }                                        => {
            syscall_trigger_int(SyscallOpCode::Exit as usize, status, 0, 0, 0)
        },
        Syscall::Open { path_ptr, mode, ret_ptr }                       => {
            syscall_trigger_int(SyscallOpCode::Open as usize,
                                path_ptr, mode, ret_ptr, 0)
        },
        Syscall::Read { fd, buf_ptr, buf_len, ret_ptr }                 => {
            syscall_trigger_int(SyscallOpCode::Read as usize,
                                fd, buf_ptr, buf_len, ret_ptr)
        },
        Syscall::Write { fd, buf_ptr, buf_len, ret_ptr }                => {
            syscall_trigger_int(SyscallOpCode::Write as usize,
                                fd, buf_ptr, buf_len, ret_ptr);
        },
        Syscall::Exec { fd, cmd_buf_ptr, buf_len, ret_ptr }               => {
            syscall_trigger_int(SyscallOpCode::Exec as usize,
                                fd, cmd_buf_ptr, buf_len, ret_ptr);
        },
        Syscall::Close { fd }                                           => {
            syscall_trigger_int(SyscallOpCode::Close as usize , fd, 0, 0, 0);
        }
    }
}

fn syscall_trigger_int(opcode: usize,
                        arg0: usize, arg1: usize, arg2: usize, arg3: usize) {
    unsafe{
        asm!(
            "int 0x20", // See boot.S
            in("rax") opcode,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            in("rcx") arg3,
        );
    };
}
//
// Standard Runtime Library for User Programs
// Eventually this will be the basis for a Rust-compatible std.
// 

#![no_std]
extern crate alloc; 

pub mod syscall;
pub mod time;
pub mod stdio;
pub mod env;
pub mod fileio;
pub mod task;
pub mod heap;
pub mod zlib;
pub mod graphics;
pub mod audio;

use crate::stdio::*;
use crate::syscall::*;

///
/// Error handling
/// 
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum ErrorCode {
    NoError             = 0,    // No error, the operation was successful
    OutOfMemory         = 1,    // Ran out of memory
    NotSupported        = 2,   // Feature/Operation not supported/implemented
    NotFound            = 3,    // The requested resource was not found
    NotIssued           = 4,    // The request couldn't be issued
    UnexpectedEoF       = 5,    // An unexpected end of file was reached
    InvalidPID          = 6,    // Invalid Process ID
    InvalidTID          = 7,    // Invalid Task ID
    InvalidFD           = 8,    // Invalid File Descriptor
    InvalidOp           = 9,    // Invalid operation for the target resource
    InvalidBus          = 10,    // Invalid Bus (e.g., AHCI[1])
    InvalidDrive        = 11,   // Invalid drive/device
    InvalidMountPoint   = 12,   // Invalid mount point
    InvalidPath         = 13,   // Invalid path
    InvalidHandle       = 14,   // Invalid device handle
    InvalidBuffer       = 15,   // Invalid buffer pointer/length
    InvalidArgument     = 16,   // Invalid argument
    InvalidFormat       = 17,   // Invalid file/data/buffer/etc. format
    NotAllowed          = 18,   // The operation is not permitted on the resource
    OutOfBoundIO        = 19,   // The IO is out-of-bound for the resource
    IOError             = 20,   // An error occurred during IO on the resource
    Other               = 255,  // Other errors
}
impl From<usize> for ErrorCode {
    fn from(code: usize) -> Self {
        match code {
            0   => ErrorCode::NoError,
            1   => ErrorCode::OutOfMemory,
            2   => ErrorCode::NotSupported,
            3   => ErrorCode::NotFound,
            4   => ErrorCode::NotIssued,
            5   => ErrorCode::UnexpectedEoF,
            6   => ErrorCode::InvalidPID,
            7   => ErrorCode::InvalidTID,
            8   => ErrorCode::InvalidFD,
            9   => ErrorCode::InvalidOp,
            10  => ErrorCode::InvalidBus,
            11  => ErrorCode::InvalidDrive,
            12  => ErrorCode::InvalidMountPoint,
            13  => ErrorCode::InvalidPath,
            14  => ErrorCode::InvalidHandle,
            15  => ErrorCode::InvalidBuffer,
            16  => ErrorCode::InvalidArgument,
            17  => ErrorCode::InvalidFormat,
            18  => ErrorCode::NotAllowed,
            19  => ErrorCode::OutOfBoundIO,
            20  => ErrorCode::IOError,
            _   => ErrorCode::Other,
        }
    }
}

#[derive(Debug)]
pub struct Exception {
    pub code: ErrorCode,
    pub message: &'static str
}
impl Exception {
    pub const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
}

///
/// Program entry, exit and panic handling
///
#[no_mangle]
extern "C" fn _main_stub() -> ! {
    env::init_proc_env();
    unsafe extern "Rust" {unsafe fn main();}
    unsafe { main(); }
    exit(0);
    loop {}
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



/// Math functions missing in core::
/// 
pub fn sin(angle_rad: f64) -> f64 {
    // Taylor series expansion for sin(x) around 0: x - x^3/3! + x^5/5! - x^7/7! + ...
    let mut term = angle_rad; // First term (n=0)
    let mut sum = term;       // Initialize sum with the first term
    let mut n = 1;
    while term.abs() > 1e-10 { // Continue until the term is small enough
        term *= -angle_rad * angle_rad / ((n + 1) * (n + 2)) as f64; // Compute next term
        sum += term; // Add the new term to the sum
        n += 2; // Increment n by 2 for the next odd term
    }
    sum
}


#![no_std]
use rtlib::*;


#[no_mangle]
extern "C"  fn main() {
    println!("Hello, World from the user space! (Ver:{:.2})", 0.01);
    print!("Enter a line:");
    let mut buff: [u8; 64] = [0; 64];
    let cnt = read_line(&mut buff);
    println!("\nYou entered({}): {}", cnt, str::from_utf8(&buff[0..cnt]).unwrap());
    exit(0);
}



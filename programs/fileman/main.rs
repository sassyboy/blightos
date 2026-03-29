//
// BlightOS - File Manager
//
#![no_std]

use rtlib::*;
use rtlib::stdio::*;
use rtlib::fileio::*;

#[no_mangle]
fn main() {
    println!("BlightOS File Manager (Ver:{:.2})!", 0.01);
    print_free_memory();
    exit(0);
}

fn read_file(path: &str) {
    let mut buff: [u8; 64] = [0; 64];
    match fopen(path) {
        Some(fd)    => {
            // let mut total = 0;
            loop {
                let cnt = fread(fd, &mut buff);
                if cnt > 0 {
                    print!("{}", str::from_utf8(&buff[0..cnt]).unwrap());
                    // total += cnt;
                } else {
                    // print!("<<EOF - {} bytes in total>>", total);
                    break;
                }
            }
            fclose(fd);
        },
        None        => {
            print!("\nPath {} doesn't exist", path);
        }
    }
}

fn print_free_memory() {
    read_file("machine:/ram");
}


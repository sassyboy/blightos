//
// BlightOS - File Manager
//
#![no_std]

use core::time::Duration;

use rtlib::*;
use rtlib::stdio::*;
use rtlib::fileio::*;
use rtlib::task::Task;

#[no_mangle]
fn main() {
    println!("BlightOS File Manager (Ver:{:.2})!", 0.01);
    print_free_memory();
    println!("Exiting File Manager...");
    Task::sleep(Duration::from_secs(5));
}

fn read_file(path: &Path) {
    let mut buff: [u8; 128] = [0; 128];
    let fort = File::from_path(path, File::MODE_READ);
    let Ok(mut file) = fort else {
        let err = fort.err().unwrap();
        println!("Can't open {} - {:?}", path.as_str(), err);
        return;
    };
    loop {
        let rdrt = file.read(&mut buff);
        let Ok(len) = rdrt else {
            let err = rdrt.err().unwrap();
            println!("Failed to read file {} - {:?}", path.as_str(), err);
            return;
        };
        if len > 0 {
            print!("{}", str::from_utf8(&buff[0..len]).unwrap());
        } else {
            break;
        }
    }
    println!("");
}

fn print_free_memory() {
    read_file(&Path::from("machine:/ram"));
}


#![no_std]
extern crate alloc; 

use alloc::string::ToString;
use rtlib::*;
use rtlib::stdio::*;
use rtlib::fileio::*;
use rtlib::task::*;
use rtlib::syscall::SyscallRsvdFDs;
use alloc::string::String;
use alloc::format;

#[no_mangle]
extern "C" 
fn main() {
    println!("BlightOS Shell (Ver:{:.2}).", 1.0);
    print_system_resources();

    // Check disk[0-9].[0-9]:/blightos/ for the binary path
    let mut bin_path = String::new();
    for d in 0..10 {
        for p in 0..10 {
            let path = format!("disk{}.{}:/blightos/", d, p);
            if path_check(&path) {
                bin_path = path;
                break;
            }
        }
    }
    // Command buffer and current working directory
    let mut cmd_buf: [u8; 512] = [0; 512];
    let mut cwd = String::new();
    let mut path: String;

    // Shell prompt loop
    loop {
        print!("\n{} > ", cwd);
        let cnt = read_line(&mut cmd_buf);
        let cmd = str::from_utf8(&cmd_buf[0..cnt]).unwrap();
        if      cmd.trim().is_empty() {
            // Do nothing
        } else if cmd.starts_with("ls ") {
            println!("");
            path = make_full_path(cwd.as_str(), &cmd[3..cnt], true);
            exec_ls(path.as_str());
        }  else if cmd.eq("ls") {
            println!("");
            exec_ls(cwd.as_str());
        } else if cmd.eq("cd ..") {
            // Go back
            if let Some(rslash) = cwd[..cwd.len() - 1].rfind("/") {
                cwd = cwd[..rslash + 1].to_string();
            }
        }else if   cmd.starts_with("cd ") {
            let path = make_full_path(cwd.as_str(), &cmd[3..cnt], true);
            if path_check(path.as_str()) {
                cwd = path;
            } else {
                print!("\nPath {} doesn't exist", path);
            }
        } else if   cmd.starts_with("txtdump ") {
            let path = make_full_path(cwd.as_str(), &cmd[8..cnt], false);
            println!("");
            exec_textdump(path.as_str());
        } else if   cmd.starts_with("hexdump ") {
            let path = make_full_path(cwd.as_str(), &cmd[8..], false);
            println!("");
            exec_hexdump(path.as_str());
        } else if cmd.starts_with("run ") {
            let path = make_full_path(cwd.as_str(), &cmd[4..], false);
            println!("");
            run_executable(path.as_str());
        } else if   cmd.starts_with("exit") {
            break;
        } else if   cmd.starts_with("cls") {
            stdio_clear_screen();
        } else if   cmd.starts_with("reboot") {
            exec_reboot();
        } else if   cmd.starts_with("ktest") {
            exec_ktest();
        }
        else if   cmd.starts_with("help"){
            println!("");
            print_help();
        } else {
            // Try to find a binary with the <cmd>.elf under the blightos
            // directory and run that if it exists
            if path_check(format!("{}{}.box", cwd, cmd.trim()).as_str()) {
                // Run the binary from the current directory
                run_executable(format!("{}{}.box", cwd, cmd.trim()).as_str());
            } else if path_check(format!("{}{}.box", bin_path, cmd.trim()).as_str()) {
                // Run the binary from the default binary path
                run_executable(format!("{}{}.box", bin_path, cmd.trim()).as_str());
            } else {
                println!("\n{} is not a valid command.", cmd);
                print_help();
            }
        }
    }
    exit(0);
}

fn print_help() {
    print!(
        "help            Prints this message\n\
         cd              Changes the current directory E.g: cd disk0.0: or cd ..\n\
         ls              Lists directories/files/devices under the current path\n\
         ls [path]       Similar to ls but looks under the current-directory/path\n\
         run [path]      Executes the file located in the path as a command\n\
         [command]       Executes the /blightos/command.box if it exists\n\
         txtdump [path]  Reads the file located in the path and prints its content\n\
         hexdump [path]  Similar to txtdump but in HEX\n\
         exit            Ends the shell program\n\
         cls             Clears the screen\n\
         reboot          Reboots the machine\n\
         test            Performs self-test from the user-space\n\
         ktest           Performs the kernel's self-test"
    );
}


fn print_system_resources() {
    let mut buff: [u8; 64] = [0; 64];
    let mut cnt = fread(SyscallRsvdFDs::SystemResources as usize, &mut buff);  
    println!("Available system resources ({}):", cnt);
    if cnt > 0 && buff[cnt-1] == b'\n' {
        cnt -= 1;
    }
    print!("{}", str::from_utf8(&buff[0..cnt]).unwrap());
}

fn path_check(path: &str) -> bool {
    match fopen(path) {
        None        => {
            false
        },
        Some(fd)   => {
            fclose(fd);
            true
        }
    }
}

fn exec_ls(path: &str) {
    let mut buff: [u8; 512] = [0; 512];

    match fopen(path) {
        None        => {
            print!("Path {} doesn't exist", path);
        },
        Some(fd)    => {
            // println!("Opened {} with FD {}", path, fd);
            let cnt = fenum(fd, &mut buff);
            // println!("Enum returned {} bytes:", cnt);
            if cnt > 0 {
                if buff[cnt-1] == b'\n' {
                    print!("{}", str::from_utf8(&buff[0..cnt-1]).unwrap());
                } else {
                    println!("{}", str::from_utf8(&buff[0..cnt]).unwrap());
                }
                
            }
            
            fclose(fd);
        }
    }

}

fn exec_textdump(path: &str) {
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

fn exec_hexdump(path: &str) {
    let mut buff: [u8; 16] = [0; 16];
    match fopen(path) {
        Some(fd)    => {
            let mut offset = 0;
            loop {
                let cnt = fread(fd, &mut buff);
                if cnt > 0 {
                    print!("{:08X}  ", offset);
                    for i in 0..cnt {
                        print!("{:02X} ", buff[i]);
                    }
                    for _i in cnt..16 {
                        print!("   ");
                    }
                    print!("    ");
                    for i in 0..cnt {
                        let b = buff[i];
                        if b.is_ascii_graphic() || b == b' ' {
                            print!("{}", b as char);
                        } else {
                            print!(".");
                        }
                    }
                    println!("");
                    offset += cnt;
                } else {
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

fn run_executable(path: &str) {
    println!("\nLaunching {} ...", path);
    if let Some(proc) = Process::spawn(path) {
        proc.join();
    } else {
        println!("\nFailed to execute {}", path);
    }
}

fn make_full_path(cur_dir: &str, path: &str, dir: bool)  -> String {
    let mut out = String::new();
    if path.starts_with("/") {
        // FUll address from the start of the mount point
        if let Some(collon) = cur_dir.find(":") {
            out.push_str(&cur_dir[..collon + 1]);
            out.push_str(path);
        } else {
            out.push_str(path);
        }
    } else if let Some(_) = path.find(":") {
        // Absolute address (includes the mount-point name)
        out.push_str(path);
    } else {
        // Address relative to the current directory
        out.push_str(cur_dir);
        if !cur_dir.ends_with("/") {
            out.push('/');
        }
        out.push_str(path);
    }
    if dir && !out.ends_with("/") {
        out.push_str("/");
    }
    out
}

fn exec_reboot() {
    match fopen("machine:/") {
        Some(fd) => {
            let mut buf: [u8; 8] = [0; 8];
            fexec(fd, 1, &mut buf);
            fclose(fd);
        },
        None => {
            print!("Could not open the machine:/ file");
        }
    }
}

fn exec_ktest() {
    match fopen("machine:/") {
        Some(fd) => {
            let mut buf: [u8; 8] = [0; 8];
            fexec(fd, 2, &mut buf);
            fclose(fd);
        },
        None => {
            print!("Could not open the machine:/ file");
        }
    }
}



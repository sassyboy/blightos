//
// BlightOS - User-space text-based shell program
//
#![no_std]
extern crate alloc;
use alloc::string::ToString;
use rtlib::*;
use rtlib::env::*;
use rtlib::stdio::*;
use rtlib::fileio::*;
use rtlib::task::*;
use rtlib::syscall::SyscallRsvdFDs;
use alloc::format;
use alloc::vec::Vec;

#[no_mangle]
fn main() {
    println!("BlightOS Shell (Ver:{:.2}).", 1.0);
    print_system_resources();
    let Ok(bin_path) = current_dir() else {
        println!("Failed to get the current directory!. Exiting...");
        exit(1);
        return;
    };

    // Shell prompt loop
    let mut cmd_buf: [u8; 512] = [0; 512];

    loop {
        let cwd = current_dir().unwrap_or_else(|_| "".to_string());
        print!("{} > ", cwd);
        let cnt = read_line(&mut cmd_buf);
        let cmd = str::from_utf8(&cmd_buf[0..cnt]).unwrap();
        println!("");
        if      cmd.trim().is_empty() {
            
        } else if cmd.starts_with("ls ") {
            exec_ls(&Path::from(&cmd[3..cnt]));
        }  else if cmd.eq("ls") {
            exec_ls(&Path::from(cwd.as_str()));
        } else if cmd.eq("cd ..") {
            // Go back
            if let Some(rslash) = cwd[..cwd.len() - 1].rfind("/") {
                let _ = set_current_dir(cwd[..rslash + 1].as_ref());
            }
        }else if   cmd.starts_with("cd ") {
            let path = Path::from(&cmd[3..cnt]);
            let fort = File::from_path(&path, File::MODE_READ);
            let Ok(file) = fort else {
                let e = fort.err().unwrap();
                println!("Path {} doesn't exist - {:?}", path.as_str(), e);
                continue;
            };
            if !file.is_dir() {
                println!("Path {} is not a directory", path.as_str());
            } else {
                let _ = set_current_dir(path.as_str());
            }
        } else if   cmd.starts_with("rd ") {
            let path = Path::from(&cmd[3..cnt]);
            exec_textdump(&path);
        } else if   cmd.starts_with("hexdump ") {
            let path = Path::from(&cmd[8..]);
            exec_hexdump(&path);
        } else if   cmd.starts_with("wr ") {
            let parts: Vec<&str> = cmd[3..].splitn(2, ' ').collect();
            if parts.len() < 2 {
                println!("Invalid command format. Usage: wr [path] [text]");
                continue;
            }
            let path = Path::from(parts[0]);
            let text = parts[1];
            exec_write(&path, text);
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
            print_help();
        } else {
            // Try to find a binary with the <cmd>.box in the current path, or
            // the blightos directory and run that if it exists
            // Also passes any arguments after the command to the new process
            let cmd_trim = cmd.trim();
            let cmd_exec = if let Some(space_idx) = cmd_trim.find(' ') {
                &cmd_trim[..space_idx]
            } else {
                cmd_trim
            };
            let cmd_args = if let Some(space_idx) = cmd_trim.find(' ') {
                &cmd_trim[space_idx + 1..]
            } else {
                ""
            };
            // Check for the command in the current directory
            let exec_path;
            if cmd_exec.ends_with(".box") {
                exec_path = Path::from(cmd_exec);
            } else {
                exec_path = Path::from(&format!("{}.box", cmd_exec));
            }
            if let Ok(_) = File::from_path(&exec_path, File::MODE_RX) {
                // Run the binary from the current directory
                run_executable(exec_path.as_str(), cmd_args);
                continue;
            }
            // If not found, check for the command in the default binary path
            if let Some(last_slash) = exec_path.as_str().rfind('/') {
                let exec_fname = &exec_path.as_str()[last_slash + 1..];
                let exec_path = Path::from(&format!("{}{}", bin_path, exec_fname));
                let fort = File::from_path(&exec_path, File::MODE_RX);
                if let Ok(_) = fort {
                    run_executable(exec_path.as_str(), cmd_args);
                    continue;
                }
            }
            println!("{} is not a valid command. Try help", cmd);
        }
    }
    exit(0);
}

fn print_help() {
    println!(
        "cd              Changes the current directory E.g: cd disk0.0: or cd ..\n\
         ls              Lists directories/files/devices under the current path\n\
         ls [path]       Similar to ls but looks under the current-directory/path\n\
         [command]       Executes command.box in the current directory or in \
                         blightos/ if it exists\n\
         wr [path] [txt] Writes the text to the file located in the path\n\
         rd [path]       Reads the file located in the path and prints its content\n\
         hexdump [path]  Similar to rd but in HEX\n\
         exit            Ends the shell program\n\
         cls             Clears the screen\n\
         reboot          Reboots the machine\n\
         test            Performs self-test from the user-space\n\
         ktest           Performs the kernel's self-test"
    );
}


fn print_system_resources() {
    let mut buff: [u8; 256] = [0; 256];
    match fread(SyscallRsvdFDs::SystemResources as usize, 0, &mut buff) {
        Ok(mut cnt) => {
            println!("Available system resources ({}):", cnt);
            if cnt > 0 && buff[cnt-1] == b'\n' {
                cnt -= 1;
            }
            println!("{}", str::from_utf8(&buff[0..cnt]).unwrap());
        }
        Err(e) => {
            println!("Failed to read system resources - {:?}", e);
            return;
        }
    }
}

fn exec_ls(path: &Path) {
    let mut buff: [u8; 512] = [0; 512];

    match File::from_path(path, File::MODE_READ) {
        Ok(dir) => {
            match dir.enum_dir(&mut buff) {
                Ok(cnt) => {
                    if cnt > 0 && buff[cnt-1] == b'\n' {
                        // It already ends with a newline
                        print!("{}", str::from_utf8(&buff[0..cnt]).unwrap());  
                    } else {
                        println!("");
                    }
                },
                Err(e) => {
                    println!("Failed to read directory {} - {:?}",
                                                path.as_str(), e);
                }
            }
        },
        Err(e) => {
            println!("Path {} doesn't exist - {:?}", path.as_str(), e);
        }
    }
}

fn exec_textdump(path: &Path) {
    let mut buff: [u8; 64] = [0; 64];
    let fort = File::from_path(path, File::MODE_READ);
    let Ok(mut file) = fort else {
        let e = fort.err().unwrap();
        println!("Can't open {} - {:?}", path.as_str(), e);
        return;
    };
    loop {
        let rdrt = file.read(&mut buff);
        let Ok(len) = rdrt else {
            let e = rdrt.err().unwrap();
            println!("Can't read from {} - {:?}", path.as_str(), e);
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

fn exec_hexdump(path: &Path) {
    let mut buff: [u8; 16] = [0; 16];
    let fort = File::from_path(path, File::MODE_READ);
    let Ok(mut file) = fort else {
        let e = fort.err().unwrap();
        println!("Can't open {} - {:?}", path.as_str(), e);
        return;
    };
    let mut offset: usize = 0;
    loop {
        let rdrt = file.read(&mut buff);
        let Ok(cnt) = rdrt else {
            let e = rdrt.err().unwrap();
            println!("Can't read from {} - {:?}", path.as_str(), e);
            return;
        };
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
    println!("");
}

fn exec_write(path: &Path, text: &str) {
    let fort = File::from_path(path, File::MODE_WRITE);
    let Ok(mut file) = fort else {
        let e = fort.err().unwrap();
        println!("Can't open {} - {:?}", path.as_str(), e);
        return;
    };
    let wrrt = file.write(text.as_bytes());
    let Ok(len) = wrrt else {
        let e = wrrt.err().unwrap();
        println!("Can't write to {} - {:?}", path.as_str(), e);
        return;
    };
    println!("Successfully wrote {} bytes to {}", len, path.as_str());
}

fn run_executable(path: &str, args: &str) {
    let cmd_line;
    if args.is_empty() {
        cmd_line = path.to_string();
        println!("Launching {} ...", path);
    } else {
        cmd_line = format!("{} {}", path, args);
        println!("Launching {} args: '{}' ...", path, args);
    }
    
    if let Some(proc) = Process::spawn(&cmd_line) {
        proc.join();
    } else {
        println!("Failed to execute {}", path);
    }
}

fn exec_reboot() {
    let fort = File::from_path(&Path::from("machine:/"), File::MODE_EXEC);
    let Ok(file) = fort else {
        let e = fort.err().unwrap();
        println!("Can't open the machine:/ - {:?}", e);
        return;
    };
    let mut buf: [u8; 8] = [0; 8];
    let fxrt = file.exec(1, &mut buf);
    let Ok(func_ret) = fxrt else {
        let e = fxrt.err().unwrap();
        println!("Failed to execute reboot - {:?}", e);
        return;
    };
    println!("Reboot function returned: {}", func_ret);
}

fn exec_ktest() {
    let fort = File::from_path(&Path::from("machine:/"), File::MODE_EXEC);
    let Ok(file) = fort else {
        let e = fort.err().unwrap();
        println!("Can't open the machine:/ - {:?}", e);
        return;
    };
    let mut buf: [u8; 8] = [0; 8];
    let fxrt = file.exec(2, &mut buf);
    let Ok(func_ret) = fxrt else {
        let e = fxrt.err().unwrap();
        println!("Failed to execute kernel test - {:?}", e);
        return;
    };
    println!("Kernel test function returned: {}", func_ret);
}



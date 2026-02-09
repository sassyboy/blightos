#![no_std]
use rtlib::*;
use rtlib::stdio::*;
use rtlib::fileio::*;
use rtlib::task::*;
use rtlib::syscall::SyscallRsvdFDs;

#[no_mangle]
extern "C" 
fn main() {
    let mut cmd_buf: [u8; 128] = [0; 128];
    println!("BlightOS Shell (Ver:{:.2}).", 0.01);
    print_system_resources();

    let mut cwd_buf_len = 0;
    let mut cwd_buf: [u8; 96] = [0; 96];
    let mut full_path: [u8; 128] = [0; 128];

    loop {
        let cwd = str::from_utf8(&cwd_buf[0..cwd_buf_len]).unwrap();
        print!("\n{} > ", cwd);
        let cnt = read_line(&mut cmd_buf);
        let cmd = str::from_utf8(&cmd_buf[0..cnt]).unwrap();

        if      cmd.trim().is_empty(){
            // Do nothing
        } else if cmd.starts_with("ls ") {
            println!("");
            let path = make_full_path(cwd, &cmd[3..], &mut full_path, true);
            exec_ls(path);
        }  else if cmd.eq("ls") {
            println!("");
            exec_ls(cwd);
        } else if cmd.eq("cd ..") {
            // Go back
            if let Some(rslash) = cwd[..cwd_buf_len - 1].rfind("/") {
                cwd_buf_len = rslash + 1;
            }
        }else if   cmd.starts_with("cd ") {
            let path;
            let mut cwd_ul = cwd.len();
            if cmd_buf[3] == b'/' {
                // Address from the start of the mount point
                if let Some(collon) = cwd.find(":") {
                    cwd_ul = collon + 1;
                }
            } else if let Some(_) = cmd.find(":") {
                // Address includes the mount point name => treat it as full
                cwd_ul = 0;
            }
            path = make_full_path(&cwd[..cwd_ul], &cmd[3..], &mut full_path, true);
            if path_check(path) {
                cwd_buf_len = path.len();
                cwd_buf[0..cwd_buf_len].copy_from_slice(&full_path[..cwd_buf_len]);
            } else {
                print!("\nPath {} doesn't exist", path);
            }
        } else if   cmd.starts_with("txtdump ") {
            let path = make_full_path(cwd, &cmd[8..], &mut full_path, false);
            println!("");
            exec_textdump(path);
        } else if   cmd.starts_with("exit") {
            exit(0);
        } else if   cmd.starts_with("reboot") {
            exec_reboot();
        } else if   cmd.starts_with("test") {
            exec_test();
        } else if   cmd.starts_with("ktest") {
            exec_ktest();
        }
        else if   cmd.starts_with("help"){
            println!("");
            print_help();
        } else {
            println!("\n{} is not a valid command.", cmd);
            print_help();
        }
    }
}

fn print_help() {
    print!  ("help          Prints this message\n\
              cd            Changes the current director. E.g: cd disk0.0: or cd ..\n\
              ls            Lists directories/files/devices under the current path\n\
              ls path       Similar to ls but looks under the current-directory/path\n\
              txtdump path  Reads the file located in the path and prints its content\n\
              hexdump path  Similar to txtdump but in HEX\n\
              exit          Ends the shell program\n\
              reboot        Reboots the machine\n\
              test          Performs self-test from the user-space\n\
              ktest         Performs the kernel's self-test"
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

fn make_full_path<'a>(cur_dir: &'a str, path: &'a str, out: &'a mut [u8],
                                                        dir: bool)  -> &'a str {
    let c = cur_dir.as_bytes();
    let p = path.as_bytes();
    out[0..c.len()].copy_from_slice(c);
    out[c.len()..(c.len()+p.len())].copy_from_slice(p);
    if dir && out[c.len()+p.len()-1] != b'/' {
        out[c.len()+p.len()] = b'/';
        return str::from_utf8(&out[0..(c.len()+p.len()+1)]).unwrap();
    }
    str::from_utf8(&out[0..(c.len()+p.len())]).unwrap()
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

fn exec_test() {
    println!("\nCurrent Task Information:");
    let t = Task::current();
    print!("TID: {}, PID: {}, name: {}", t.tid, t.pid, t.name());
}

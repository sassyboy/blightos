///
/// Environment information/variables and program arguments
/// 

use crate::{Exception, ErrorCode};
use crate::task::{Process, Spinlock};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

static CUR_EXE: Spinlock<String> = Spinlock::new(String::new());
static CUR_DIR: Spinlock<String> = Spinlock::new(String::new());
static ARGS: Spinlock<Vec<String>> = Spinlock::new(Vec::new());

pub fn current_exe() -> Result<String, Exception> {
    let exe = CUR_EXE.lock();
    if exe.is_empty() {
        Err(Exception::new(ErrorCode::NotFound, "Current executable not set"))
    } else {
        Ok(exe.clone())
    }
}

pub fn current_dir() -> Result<String, Exception> {
    let dir = CUR_DIR.lock();
    if dir.is_empty() {
        Err(Exception::new(ErrorCode::NotFound, "Current directory not set"))
    } else {
        Ok(dir.clone())
    }
}

pub fn args() -> Vec<String>{
    let args = ARGS.lock();
    args.clone()
}

pub fn set_current_dir(path: &str) -> Result<(), Exception> {
    let mut dir = CUR_DIR.lock();
    dir.clear();
    dir.push_str(path);
    Ok(())
}

pub fn init_proc_env() {
    let mut proc = Process::current();
    proc.get_info();

    let exec_path;
    let cur_dir;
    let args_vec: Vec<String>;
    let cmd_parts = proc.cmd_line.split(' ').collect::<Vec<&str>>();
    if !cmd_parts.is_empty() {
        exec_path = cmd_parts[0];
        cur_dir = match exec_path.rfind('/') {
            Some(idx) => &exec_path[..idx+1],
            None => "."
        };
        CUR_EXE.lock().clear();
        CUR_EXE.lock().push_str(exec_path);
        CUR_DIR.lock().clear();
        CUR_DIR.lock().push_str(cur_dir);
        ARGS.lock().clear();
        if cmd_parts.len() > 1 {
            args_vec = cmd_parts[1..].iter().map(|s| s.to_string()).collect();
            ARGS.lock().extend(args_vec);
        }
    }
}
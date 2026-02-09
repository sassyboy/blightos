//
// Multi-processing/Multi-threading API
//
use crate::syscall::*;

pub struct Task {
    pub tid:        usize,
    pub pid:        usize,
    tname:          [u8; 64], // Null-terminated
}

impl Task {
    // pub fn spawn(func: fn()) -> Self {
    //     Self {
    //         pid:    0,
    //         tid:    0
    //     }
    // }

    pub fn current() -> Self {
        let mut syscall_args = TaskControlCurrentArguments {
            tid:        0,
            pid:        0,
            name:       [0 as u8; 64],
        };
        let mut retval: usize = 0;

        syscall(Syscall::TaskControl {
            opcode: TaskControlOpCode::Current as usize,
            args: &mut syscall_args as *mut TaskControlCurrentArguments as usize,
            ret_code: &mut retval as *mut usize as usize
        });
        
        if retval != size_of::<TaskControlCurrentArguments>() {
            panic!("Bug in Syscall::TaskControl/Current");
        }
        Self {
            tid:    syscall_args.tid,
            pid:    syscall_args.pid,
            tname:  syscall_args.name
        }
    }

    pub fn name(&self) -> &str {
        let strlen = self.tname.iter().position(|&b| b == 0)
                                                .unwrap_or(self.tname.len());
        str::from_utf8(&self.tname[..strlen]).unwrap()
    }


}

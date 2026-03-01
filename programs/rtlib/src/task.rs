//
// Multi-processing/Multi-threading API
//
extern crate alloc; 
use alloc::string::*;
use crate::syscall::*;

pub struct Task {
    pub tid:        usize,
    pub pid:        usize,
    tname:          [u8; 64], // Null-terminated
}

impl Task {
    const MAX_NAME_LEN: usize = 64;

    //
    // Member functions
    //
    pub fn name(&self) -> &str {
        let strlen = self.tname.iter().position(|&b| b == 0)
                                                .unwrap_or(self.tname.len());
        str::from_utf8(&self.tname[..strlen]).unwrap()
    }

    //
    // Static functions
    //
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

    pub fn  current_cpu() -> usize {
        let mut retval: usize = 0;

        syscall(Syscall::TaskControl {
            opcode: TaskControlOpCode::CurrentCpu as usize,
            args: 0,
            ret_code: &mut retval as *mut usize as usize
        });
        
        retval
    }

    pub fn spawn(func: fn(usize), func_arg: usize, name: &str) -> Option<Self> {
        if name.len() >= Self::MAX_NAME_LEN {
            return None; // Name too long
        }

        let mut syscall_args = TaskControlSpawnArguments {
            func_ptr:   func as usize,
            func_arg:   func_arg,
            name:       [0 as u8; 64],
            name_len:   name.len(),
            tid:        0,
            pid:        0
        };
        let mut retval: usize = 0;

        let name_bytes = name.as_bytes();
        syscall_args.name[..name_bytes.len()].copy_from_slice(name_bytes);

        syscall(Syscall::TaskControl {
            opcode: TaskControlOpCode::Spawn as usize,
            args: &mut syscall_args as *mut TaskControlSpawnArguments as usize,
            ret_code: &mut retval as *mut usize as usize
        });
        
        if retval != size_of::<TaskControlSpawnArguments>() {
            panic!("Bug in Syscall::TaskControl/Spawn");
        }
        Some(Self {
            tid:    syscall_args.tid,
            pid:    syscall_args.pid,
            tname:  syscall_args.name,
        })
    }

    pub fn join(tid: usize) {
        let mut syscall_args = TaskControlJoinArguments {
            tid,
            joined: false
        };
        let mut retval: usize = 0;

        syscall(Syscall::TaskControl {
            opcode: TaskControlOpCode::Join as usize,
            args: &mut syscall_args as *mut TaskControlJoinArguments as usize,
            ret_code: &mut retval as *mut usize as usize
        });
        
        if retval != size_of::<TaskControlJoinArguments>() {
            panic!("Syscall::TaskControl/Join");
        }
    }

    pub fn yield_now() {
        syscall(Syscall::TaskControl {
            opcode: TaskControlOpCode::Yield as usize,
            args: 0,
            ret_code: 0
        });
    }

    pub fn sleep(duration: core::time::Duration) {
        syscall(Syscall::TaskControl {
            opcode: TaskControlOpCode::Sleep as usize,
            args: &duration as *const core::time::Duration as usize,
            ret_code: 0
        });
    }

}

#[derive(Debug)]
pub struct Process {
    pub pid:                usize,
    pub main_tid:           usize,
    // The following fields are only populated/update when get_info() is called,
    // and are not guaranteed to be up-to-date at all times.
    pub name:               String,
    pub task_count:         usize,
    pub fd_count:           usize,
    pub img_base:           usize,
    pub img_size:           usize,
    pub heap_base:          usize,
    pub heap_size:          usize,
    pub stack_top:          usize,
    pub total_mem_usage:    usize,
    pub meta_mem_usage:     usize,
}

impl Process {
    pub fn current() -> Self {
        let mut syscall_args = ProcCtlCurrentArgs {
            pid: 0,
            main_tid: 0
        };
        let mut retval: usize = 0;
        syscall(Syscall::ProcControl {
            opcode: ProcCtlOpCode::Current as usize,
            args: &mut syscall_args as *mut ProcCtlCurrentArgs as usize,
            ret_code: &mut retval as *mut usize as usize
        });
        if retval != size_of::<ProcCtlCurrentArgs>() {
            panic!("Bug in Syscall::ProcControl/Current");
        }
        Self { 
            pid:            syscall_args.pid,
            main_tid:       syscall_args.main_tid,
            name:           String::new(),
            task_count:     0,
            fd_count:       0,
            img_base:       0,
            img_size:       0,
            heap_base:      0,
            heap_size:      0,
            stack_top:      0,
            total_mem_usage: 0,
            meta_mem_usage: 0
        }
    }

    pub fn get_info(&mut self) -> bool {
        let mut syscall_args = ProcCtlGetInfoArgs {
            pid:        self.pid,
            name:       [0; 64],
            main_tid:   0,
            task_count: 0,
            fd_count:   0,
            img_base:   0,
            img_size:   0,
            heap_base:  0,
            heap_size:  0,
            stack_top:  0,
            total_mem_usage:    0,
            meta_mem_usage:     0
        };
        let mut retval: usize = 0;
        syscall(Syscall::ProcControl {
            opcode:     ProcCtlOpCode::GetInfo as usize,
            args:       &mut syscall_args as *mut ProcCtlGetInfoArgs as usize,
            ret_code:   &mut retval as *mut usize as usize
        });
        if retval != size_of::<ProcCtlGetInfoArgs>() {
            return false; // Failed to get info
        }
        
        // Update the process info with the data returned from the kernel
        self.name.clear();
        self.name.push_str(str::from_utf8(&syscall_args.name).unwrap()
                                            .trim_end_matches(char::from(0)));
        self.task_count =       syscall_args.task_count;
        self.fd_count =         syscall_args.fd_count;
        self.img_base =         syscall_args.img_base;
        self.img_size =         syscall_args.img_size;
        self.heap_base =        syscall_args.heap_base;
        self.heap_size =        syscall_args.heap_size;
        self.stack_top =        syscall_args.stack_top;
        self.total_mem_usage =  syscall_args.total_mem_usage;
        self.meta_mem_usage =   syscall_args.meta_mem_usage;
        true
    }

    pub fn spawn(exec_path: &str) -> Option<Self> {
        let mut syscall_args = ProcCtlSpawnArgs {
            path_ptr: 0,
            path_len: exec_path.len(),
            pid: 0,
            m_tid: 0
        };
        let mut retval: usize = 0;
        let path_bytes = exec_path.as_bytes();
        syscall_args.path_ptr = path_bytes.as_ptr() as usize;

        syscall(Syscall::ProcControl {
            opcode: ProcCtlOpCode::Spawn as usize,
            args: &mut syscall_args as *mut ProcCtlSpawnArgs as usize,
            ret_code: &mut retval as *mut usize as usize
        });

        if retval != size_of::<ProcCtlSpawnArgs>() {
            return None; // Failed to spawn process
        }
        Some(Self {
                pid:            syscall_args.pid,
                main_tid:       syscall_args.m_tid,
                name:           String::new(),
                task_count:     0,
                fd_count:       0,
                img_base:       0,
                img_size:       0,
                heap_base:      0,
                heap_size:      0,
                stack_top:      0,
                total_mem_usage: 0,
                meta_mem_usage: 0
            }
        )
    }

    pub fn join(&self) {
        Task::join(self.main_tid);
    }
}


//
// Synchronization
//

use core::{
 cell::UnsafeCell, ops::{Deref, DerefMut}, sync::atomic::{AtomicBool, Ordering}
};

pub struct Spinlock<T> {
    is_locked:  AtomicBool,
    data:       UnsafeCell<T>,
}

pub struct SpinlockCriticalSection<'a, T: 'a> {
    sl:     &'a Spinlock<T>,
}

unsafe impl<T> Send for Spinlock<T> {}
unsafe impl<T> Sync for Spinlock<T> {}

impl<T> Spinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            is_locked:  AtomicBool::new(false),
            data:       UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinlockCriticalSection<'_, T> {
        loop {
            if self.is_locked.swap(true, Ordering::AcqRel) == false {
                return SpinlockCriticalSection { sl: self };
            }

            while self.is_locked.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }

    pub fn try_lock(&self) -> Option<SpinlockCriticalSection<'_, T>> {
        if !self.is_locked.swap(true, Ordering::AcqRel) {
            // is_locked was false and now we have atomically swapped it to true,
            // so no one else has access to this data.
            return Some(SpinlockCriticalSection { sl: self});
        }
        None
    }
}

impl<'a, T: 'a> Drop for SpinlockCriticalSection<'a, T> {
    fn drop(&mut self) {
        self.sl.is_locked.store(false, Ordering::Release);
    }
}

impl<'a, T> Deref for SpinlockCriticalSection<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.sl.data.get() }
    }
}

impl<'a, T> DerefMut for SpinlockCriticalSection<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.sl.data.get() }
    }
}
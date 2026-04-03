//
// BlightOS Kernel
//
// Machine Device Driver
//
// This driver registers a mount-point (machine:) to accept file operations from
// the user-space to perform system-wide funtions, e.g., reboot, kernel's
// self-test, etc
//

use core::cmp::min;
use alloc::vec::Vec;
use alloc::{format, slice};
use alloc::string::String;
use crate::arch::cpu_count;
use crate::fs::{DirectoryEntry, FileOperation, MountPoint};
use crate::mem::phys::{PHY_FRAME_SIZE, pmm_num_free_frames, pmm_num_total_frames};
use crate::sched::Task;
use crate::{test, util::*};
use crate::Error;

pub struct Machine {

}

impl Machine {
    // Handles 0 to cpu_count() - 1 are readable files returning stats of each CPU
    pub const DEV_HANDLE_FIRST_CPU:     usize = 0;

    // Reading from the RAM device returns the # of free physical frames
    pub const DEV_HANDLE_RAM:           usize = 1000;
    // Device Handle 100 supports system-wide functions (fexec) below:
    pub const DEV_HANDLE_DEFAULT:       usize = 1001;
    pub const FUNC_REBOOT:              usize = 1;
    pub const FUNC_KERNEL_SELF_TEST:    usize = 2;

    pub fn enumerate() -> usize {
        let mnt_obj = MountPoint {
            name:       String::from("machine"),
            fops:       Self::fops_handler
        };
        if MountPoint::mount(mnt_obj) {
            return 1;
        }
        0
    }

    pub fn post_enum() {

    }

    pub fn release(_dev_id: usize) {

    }

    fn fops_handler(op: FileOperation) -> Result<usize, Error> {
        match op {
            FileOperation::Exec { hnd, func, buff: _ }         => {
                if hnd != Self::DEV_HANDLE_DEFAULT {
                    return Err(error!(ErrorCode::InvalidHandle));
                }
                match func {
                    Self::FUNC_REBOOT           => {
                        Self::exec_reboot();
                    },
                    Self::FUNC_KERNEL_SELF_TEST => {
                        Self::exec_ktest();
                    },
                    _       => {
                        return Err(error!(ErrorCode::InvalidHandle));
                    }
                }
            },
            FileOperation::Open { full_path, mode: _, dent } => {
                let mpath = MountPoint::device_relative_path(full_path);
                if mpath.eq("/") {
                    dent.name = String::from("");
                    dent.size = 0;
                    dent.flags = DirectoryEntry::DEV_RX_DIR_FLAGS;
                    return Ok(Self::DEV_HANDLE_DEFAULT);
                } else if mpath.starts_with("/cpu") {
                    if let Ok(cpu_num) = mpath[4..].parse::<usize>() {
                        if cpu_num < cpu_count() {
                            dent.name = format!("cpu{}", cpu_num);
                            dent.size = 0;
                            dent.flags = DirectoryEntry::DEV_R_FILE_FLAGS;
                            return Ok(cpu_num);
                        }
                    }
                    return Err(error!(ErrorCode::InvalidPath));
                } else if mpath.eq("/ram") {
                    dent.name = String::from("ram");
                    dent.size = 128; // Enough to hold the stats string
                    dent.flags = DirectoryEntry::DEV_R_FILE_FLAGS;
                    return Ok(Self::DEV_HANDLE_RAM);
                } else {
                    return Err(error!(ErrorCode::InvalidPath));
                }
                
            },
            FileOperation::Close { hnd }                    => {
                if hnd == Self::DEV_HANDLE_DEFAULT ||
                   hnd == Self::DEV_HANDLE_RAM {
                    return Ok(0);
                }
                return  Err(error!(ErrorCode::InvalidHandle));
            },
            FileOperation::Enum { hnd, out }                => {
                return Self::fenum(hnd, out);
            }
            FileOperation::Read { hnd, off, buff }          => {
                return Self::fread(hnd, off, buff);
            }
            _                                               => {
                return Err(error!(ErrorCode::InvalidOp));
            }
        }
        Ok(0)
    }

    fn exec_reboot() {
        klog!("Rebooting the machine...\n");
        // TODO - Call the release handler of the device drivers
        // Perform the low level reboot
        crate::arch::machine_reboot();
    }
    fn exec_ktest() {
        Task::spawn_on_cpu(|_arg: usize| {
            test::kself_test();
        }, 0, 0, String::from("Machine::ktest"));
        
    }

    fn fenum(hnd: usize, out: &mut Vec<DirectoryEntry>) -> Result<usize, Error>{
        if hnd != Self::DEV_HANDLE_DEFAULT {
            // Only the root (machine:/) provides a list
            return Err(error!(ErrorCode::InvalidOp));
        }
        // List the CPU Entries
        let num_cpus = cpu_count();
        for i in 0..num_cpus {
            out.push(
                DirectoryEntry {
                    name: format!("cpu{}", i),
                    size:  0,
                    flags: DirectoryEntry::DEV_R_FILE_FLAGS
                }
            );
        }
        out.push(
            DirectoryEntry {
                name: String::from("ram"),
                size:  0,
                flags: DirectoryEntry::DEV_R_FILE_FLAGS
            }
        );
        Ok(out.len())
    }

    fn fread(hnd: usize, off: usize, buff: &mut [u8]) -> Result<usize, Error> {
        if hnd == Self::DEV_HANDLE_RAM {
            let out = format!(
                "Total: {} Frames, {:.3} MB\n\
                 Free : {} Frames, {:.3} MB",
                pmm_num_total_frames(),
                (pmm_num_total_frames() * PHY_FRAME_SIZE) as f64 / 0x100000 as f64,
                pmm_num_free_frames(),
                (pmm_num_free_frames() * PHY_FRAME_SIZE) as f64 / 0x100000 as f64
            );
            let len = min(out[off..].len(), buff.len());
            let ptr = out[off..].as_ptr();
            unsafe {
                buff[0..len].copy_from_slice(slice::from_raw_parts(ptr, len));
            }
            return Ok(len);
        }
        Err(error!(ErrorCode::InvalidOp))
    }


}
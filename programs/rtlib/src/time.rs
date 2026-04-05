//
// Time Management Interface
//
use crate::ErrorCode;
use crate::syscall;
use crate::syscall::*;
use core::arch::asm;

pub struct TimeStampCounter {
    tsc_freq_hz: u64,
}
impl TimeStampCounter {
    pub fn new() -> Self {
        Self {
            tsc_freq_hz: 0
        }
    }
    pub fn current_tick(&mut self) -> u64 {
        if self.tsc_freq_hz == 0 {
            let mut args = TimeCtlTscFreqArgs { tsc_freq_hz: 0 };
            let mut ret_val: usize = ErrorCode::NoError as usize;
            syscall(Syscall::TimeControl {
                opcode: TimeCtlOpCode::GetTscFreq as usize,
                args_ptr: &mut args as *mut _ as usize,
                args_len: core::mem::size_of::<TimeCtlTscFreqArgs>(),
                ret_ptr: &mut ret_val as *mut _ as usize
            });
            if ret_val != ErrorCode::NoError as usize {
                // Failed to get TSC frequency, cannot proceed with timestamping
                return 0;
            }
            self.tsc_freq_hz = args.tsc_freq_hz;
        }
        #[cfg(target_arch = "x86_64")]
        {
            let (upper, lower): (u64, u64);
            unsafe {
                asm!("rdtsc", out("rdx")upper, out("rax")lower);
            }
            (upper << 32) | lower
        }
        #[cfg(target_arch = "aarch64")]
        {
            let cntvct: u64;
            unsafe {
                asm!("mrs {}, cntvct_el0", out(reg) cntvct);
            }
            cntvct
        }
    }
    pub fn current_as_nanos(&mut self) -> u64 {
        let ticks = self.current_tick();
        if self.tsc_freq_hz == 0 {
            // Cannot convert to time without knowing frequency
            return 0;
        }
        (ticks * 1_000_000_000) / self.tsc_freq_hz
    }
    pub fn freq_hz(&self) -> u64 {
        self.tsc_freq_hz
    }
    pub fn current_as_duration(&mut self) -> core::time::Duration {
        let nanos = self.current_as_nanos();
        core::time::Duration::from_nanos(nanos)
    }
}
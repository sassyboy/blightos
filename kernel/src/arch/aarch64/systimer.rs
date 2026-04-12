// 
// BlightOS Kernel
// 
// Support module for the AARCH64 Core Timer/Counters
// Used by the scheduler
//

use core::time::Duration;
use core::arch::asm;
use crate::arch::*;
use crate::util::*;

percpu_global!{
    THIS_CPU_SYSTIMER:          SystemTimer = SystemTimer::new();
}

pub struct SystemTimer {
    mode:       SysTimerMode,
}

// Timestamp Counter Support: Using CNTPCT_EL0
//
// Note 1: Accesses to CNTPCT_EL0 may be trapped into EL1/EL2:
// If CNTKCTL_EL1.EL0PCTEN == 0, EL0 accesses to CNTPCT_EL0 are trapped
// If CNTKCTL_EL1.EL0VCTEN == 0, EL0 accesses to CNTVCT_EL0 are trapped
//
// Note 2: The frequency of the timestamp counter is reported to the OS by
//         the firmware using CNTFRQ_EL0, which can only be modified by the
//         highest implemented EL.
//
// Event/Preemption Support: Using CNTP_CVAL_EL0 (and CNTP_CTL_EL0)
// Note:
// TimerConditionMet = 
//          (((PhysicalCountInt() - Offset)[63:0] - CompareValue[63:0]) >= 0)
// Where
// PhysicalCountInt() is the physical counter value, which can be read from the
//                    CNTPCT_EL0 register when read from EL2 or EL3.
// Offset: For the EL1 physical timer, the offset value is the value held in
//         the CNTPOFF_EL2 register (Useful for virtualization)
//
// Reference: D12.2.4.1 Operation of the CompareValue views of the timers

impl SystemTimer {
    pub const fn new() -> Self {
        Self{
            mode: SysTimerMode::Disabled,
        }
    }

    fn arm_one_shot(&self, d: Duration) {
        let duration_tsc = ((d.as_nanos() as f64 / 1_000_000_000.0) * 
                                                Self::freq_hz() as f64) as u64;
        let target = Self::current_timestamp() + duration_tsc;
        // klog!("Cur = {}, target = {}, diff = {}\n",
        //     Self::current_timestamp(), target,
        //     Self::current_timestamp() - duration_tsc);
        unsafe {
            asm!(
                "msr CNTP_CVAL_EL0, {0}",
                in(reg)target
            )
        }
    }

    fn arm_periodic(&self, _p: Duration) {
        panic!("Not implemented yet!\n");
    }

    fn freq_hz() -> u64 {
        let f: u64;
        unsafe {
            asm!("mrs {0}, CNTFRQ_EL0", out(reg)f);
        }
        f
    }
    pub fn irq_pending() -> bool {
        let ctlreg: u64;
        unsafe {
            asm!(
                "mrs {0}, CNTP_CTL_EL0",
                out(reg)ctlreg
            )
        }
        ctlreg & 0x4 > 0
    }
    pub fn send_eoi() {
        unsafe {
            asm!(
                "msr CNTP_CVAL_EL0, {0}",
                in(reg)u64::MAX
            )
        }
    }

}
static mut SYSTIMER_HANDLER: IsrHandlerFn = |_|{};

impl SystemTimerTrait for SystemTimer {
    // To be called once during kernel's serialized initialization to install a
    // single IRQ handler. Every core will execute the same handler code, even
    // though each having an individual timer (and set of events)
    fn global_init(isr_callback: IsrHandlerFn) {
        unsafe {
            SYSTIMER_HANDLER = isr_callback;
        }
    }

    fn per_cpu_init() {
        unsafe {
            // Don't trap EL0 when accessing cntvct_el0
            let mut cntkctl: u64;
            asm!("mrs {0}, CNTKCTL_EL1", out(reg)cntkctl);
            cntkctl |= 0x3; // Set EL0VCTEN and EL0PCTEN to 1
            asm!("msr CNTKCTL_EL1, {0}", in(reg)cntkctl);
        }   
    }

    fn exec_handler() {
        SystemTimer::send_eoi();
        unsafe{
            SYSTIMER_HANDLER(crate::arch::cpu_id() as u16);
        }
    }
    // Per-CPU - Each CPU can configure a different mode for its timer
    fn set_mode(mode: SysTimerMode){
        // Set traps for EL0 when accessing CNTFRQ_EL0 or any of the virtual
        // or physical 
        // klog!("CPU{} - System Counter Effective Freq: {} Hz\n", cpu_id(),
        //         Self::freq_hz());
        match mode {
            SysTimerMode::OneShot   => {
                // Set the compare value to MAX, and enable the Timer
                // CNTP_CTL_EL0 = b01 (IRQMasked = 0, TimerEnabled = 1)
                unsafe {
                    asm!(
                        "msr CNTP_CVAL_EL0, {0}",
                        "msr CNTP_CTL_EL0, {1}",
                        in(reg) u64::MAX,
                        in(reg) 1 as u64
                    );
                }
                BCM2836L1IntCtl::mask_core_timer_irq(false);
            }
            SysTimerMode::Periodic  => {
                panic!("The SystemTimer doesn't support a periodic mode yet.");
            }
            SysTimerMode::Disabled  => {
                // CNTP_CTL_EL0 = b10 (IRQMasked = 1, TimerEnabled = 0)
                unsafe {
                    asm!("msr CNTP_CTL_EL0, {0}", in(reg)2 as u64);
                    BCM2836L1IntCtl::mask_core_timer_irq(true);
                }
            }
        }
        THIS_CPU_SYSTIMER.borrow_mut().mode = mode;   
    }

    // Per-CPU - Sets the period of IRQs or the next IRQ to generate depending
    // on the mode set for the timer.
    fn arm(duration: Duration) {
        let timer = THIS_CPU_SYSTIMER.borrow_mut();
        match timer.mode {
            SysTimerMode::Disabled      => {},
            SysTimerMode::OneShot       => {timer.arm_one_shot(duration);}
            SysTimerMode::Periodic      => {timer.arm_periodic(duration);}
        }
    }

    //
    // Timestamp Interface
    //
    fn frequency_hz() -> u64 {
        Self::freq_hz()
    }

    fn current_timestamp() -> u64 {
        let tsc: u64;
        unsafe {
            asm!(
                "dsb sy",
                "isb sy",  // For precise benchmarking - Should be left to the caller?
                "mrs {0}, CNTPCT_EL0",
                out(reg)tsc
            );
        }
        tsc
    }

    fn duration_to_timestamp_ticks(d: Duration) -> u64 {
        ((d.as_nanos() as f64 / 1_000_000_000.0) * Self::freq_hz() as f64) as u64
    }

    fn timestamp_to_duration(t: u64) -> Duration {
        
        Duration::from_nanos((t as f64 / 
                             (Self::freq_hz() as f64 / 1_000_000_000.0)) as u64)
    }
}
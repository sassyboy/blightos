// 
// BlightOS Kernel
// 
// Support module for the x64 Core Timer/Counters
//

use core::arch::asm;
use core::time::Duration;
use crate::arch::*;
use crate::util::*;

#[cfg(feature="debug_systimer")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[SYSTIME] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}
#[cfg(not(feature="debug_systimer"))]
macro_rules! dbg{
    ($($arg:tt)*) => { };
}

static mut X86_LAPIC_TIMER_HANDLER: IsrHandlerFn = |_| {};
percpu_global! {
    pub THIS_TSC:    X86TimeStampCounter = X86TimeStampCounter::new();
    THIS_CPU_SYSTIMER:          SystemTimer = SystemTimer::new();
}

pub struct SystemTimer {
    mode: SysTimerMode
}

impl SystemTimer {
    pub const fn new() -> Self {
        Self{
            mode: SysTimerMode::Disabled
        }
    }

    fn arm_one_shot(&self, d: Duration) {
        let tsc = THIS_TSC.borrow_mut();
        let duration_tsc = ((d.as_nanos() as f64 / 1_000_000_000.0) *
                                                    tsc.freq_hz as f64) as u64;
        let target = tsc.read() + duration_tsc;
        THIS_LAPIC.borrow_mut().set_timer(target);
    }

    fn arm_periodic(&self, _p: Duration) {
        panic!("Not implemented yet!\n");
    }
}
impl SystemTimerTrait for SystemTimer {
    // To be called once during kernel's serialized initialization to install a
    // single IRQ handler. Every core will execute the same handler code, even
    // though each having an individual timer (and set of events)
    fn global_init(isr_callback: IsrHandlerFn) {
        unsafe {
            X86_LAPIC_TIMER_HANDLER = isr_callback;
        }
    }

    fn exec_handler() {
        unsafe {
            X86_LAPIC_TIMER_HANDLER(cpu_id() as u16);
        }
    }

    // Per-CPU - Each CPU can configure a different mode for its timer
    fn set_mode(mode: SysTimerMode){
        match mode {
            SysTimerMode::OneShot   => {
                THIS_LAPIC.borrow_mut().config_timer(33, false);
            }
            SysTimerMode::Periodic  => {
                panic!("The SystemTimer doesn't support a periodic mode yet.");
            }
            SysTimerMode::Disabled  => {
                THIS_LAPIC.borrow_mut().config_timer(33, true);
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
    fn current_timestamp() -> u64 {
        cpu_read_timestamp()
    }

    fn duration_to_timestamp_ticks(d: Duration) -> u64 {
        let tsc = THIS_TSC.borrow_mut();
        ((d.as_nanos() as f64 / 1_000_000_000.0) * tsc.freq_hz as f64) as u64
    }

    fn timestamp_to_duration(t: u64) -> Duration {
        let tsc = THIS_TSC.borrow_mut();
        Duration::from_nanos((t as f64 / 
                                (tsc.freq_hz as f64 / 1_000_000_000.0)) as u64)
    }

    fn current_timestamp_as_duration() -> Duration {
        let tsc = THIS_TSC.borrow_mut();
        Duration::from_nanos( (cpu_read_timestamp() as f64 / 
                                (tsc.freq_hz as f64 / 1_000_000_000.0)) as u64)
    }
}

// For newer CPUs that support Invariant TSCs, this is going to be used
// for time-keeping and preemption interrupts
// TODO: fallback to PIT (or HPET) in case the system doesn't support this
pub struct X86TimeStampCounter {
    freq_hz:        u64,
    enabled:        bool,   // True: Can be used in the kernel: freq_hz is valid
                            //       and the frequency is invariant
    tsc_deadline:   bool,   // LAPICs can use the TSC_DEADLINE mode
}
impl X86TimeStampCounter {
    pub const fn new() -> Self {
        Self {
            freq_hz:        0,
            enabled:        false,
            tsc_deadline:   false,
        }
    }
    pub fn init(&mut self) {
        let (mut eax, ebx, mut ecx, mut edx) : (u32, u32, u32, u32);
        let (cpu_family, mut cpu_model) : (u8, u8);
        self.enabled = false;
        // Is TSC supported? CPUID.01H -> EDX[4]
        // Is TSC_DEADLINE supported? CPUID1.01H -> ECX[24]
        (eax, _, ecx, edx) = x86_cpuid_inst(0x1, 0);
        if edx & 0x10 == 0 {
            panic!("Processor too old. TSC not supported.");
        }
        if ecx & (1 << 24) == 0 {
            panic!("Processor too old. TSC_DEADLINE not supported.");
        }
        cpu_model      = ((eax & 0xF0)  >> 4) as u8;
        cpu_family     = ((eax & 0xF00) >> 8) as u8;
        if cpu_family == 0x06 || cpu_family ==  0x0F {
            // Extended model (EAX[19:16]) prepended
            cpu_model |= ((eax & 0xF0000) >> 12) as u8;
        }

        // Is the rate invariant CPUID.80000007H -> EDX.bit8 (TSC_INVARIANT)
        if x86_cpuid_max_extended_leaf() >= 0x80000007 {
            (_, _, _, edx) = x86_cpuid_inst(0x80000007, 0);
            if edx & 0x100 > 0 {
                // Derive the TSC frequency:
                // CPUID.15H: Time Stamp Counter and Nominal Core Crystal Clock
                // EAX -> Denominator
                // EBX -> Numerator
                // ECX -> Core Crysctal Freq (Could be Zero)
                // TSC_frequency = ECX * EBX/EAX
                (eax, ebx, ecx, _) = x86_cpuid_inst(0x15, 0);
                dbg!("CPUID.15H - EAX: {} EBX {} ECX: {}\n", eax, ebx, ecx);
                if ecx == 0 && cpu_family == 0x6 {
                    // Core Crystal Clock Freq is not enumerated, but we can
                    // look it up base on the model. According to Intel's SDM:
                    // Table 21-95. Nominal Core Crystal Clock Frequency
                    // 25MHz: Intel Xeon Scalable Processor Family(CPUID 06_55H)
                    // 24MHz: 6th and 7th gen Intel Core and Intel Xeon W.
                    // 19.2MHz: Next Generation Intel Atom processors based on
                    //          Goldmont Microarchitecture with CPUID signature
                    //          06_5CH (does not include Intel Xeon processors).
                    // See Tabel 2-1.
                    // CPUID Signature Values of DisplayFamily_DisplayModel
                    // For a complete list
                    // CPUID.01H -> EAX[7:4] model, EAX[11:8] family
                    ecx = match cpu_model{
                        0x55 => 25000000,
                        0x4E => 24000000,
                        0x8E => 24000000,
                        0x5C => 19200000,
                        _    => 0
                    };
                }
                if eax > 0 && ebx > 0 && ecx > 0 {
                    self.freq_hz = ((ecx as f64) * (ebx as f64/ eax as f64))
                                    as u64;
                    // TSC is supported, has a constant frequency, which is
                    // enumerated here!
                    self.enabled = true;
                    dbg!("Invariant TSC Freq: {} - EAX: {} EBX {} ECX: {}\n",
                            self.freq_hz, eax, ebx, ecx);
                } else {
                    // Approximate it
                    // Not the most precise method, but yields < 0.1% error
                    // Good enough!
                    let tm = THIS_MACHINE.lock();
                    if tm.hpet.valid {
                        tm.hpet.disable_counting();
                        tm.hpet.reset_current_count();
                        let target = tm.hpet.duration_to_ticks(Duration::from_millis(10)); 
                        let mut hpt1 = 0;
                        let tsc0 = cpu_read_timestamp();
                        tm.hpet.enable_counting();
                        while hpt1 < target {
                            hpt1 = tm.hpet.current_count();
                        }
                        // let tsc1 = cpu_read_timestamp();
                        tm.hpet.current_count();
                        tm.hpet.disable_counting();
                        let tsc1 = cpu_read_timestamp();

                        let hpet_freq = 1_000_000_000 as f64 / (tm.hpet.period_ns) as f64;
                        self.freq_hz = ( ((tsc1-tsc0) as f64 / hpt1 as f64) *
                                                            hpet_freq) as u64;
                        // Log for testing purposes
                        dbg!("Invariant TSC - HPET-Approximated Freq: {} KHz\n"
                                ,self.freq_hz / 1000);
                        // let hzdelta = core::cmp::max(self.freq_hz, approx) - 
                        //             core::cmp::min(self.freq_hz, approx);
                        // klog!("Invariant TSC {} HPET-Approximated: {} HZ, \
                        //        d, delta%:{:.4}, num: {}, den: {}, hpet_T: {}\n",
                        //         self.freq_hz, approx,
                        //         hzdelta as f64 / self.freq_hz as f64 * 100.0,
                        //         tsc1-tsc0, hpt1, tm.hpet.period_ns
                        //     );
                    } else {
                        // Resort to PIT
                        x86_pit::config_oneshot_count(100); // 100 ms count-down
                        x86_pit::start_oneshot_count();
                        let start_tsc = cpu_read_timestamp();
                        x86_pit::wait_for_oneshot_count();
                        let end_tsc = cpu_read_timestamp();
                        let tsc_overhead1 = cpu_read_timestamp();
                        self.freq_hz = (end_tsc - start_tsc - 
                                          (tsc_overhead1-end_tsc)*2) * 10;
                        dbg!("INVARIANT TSC PIT FREQ APPROX: {} HZ\n",
                                self.freq_hz);
                    }
                }
            } else {
                panic!("Processor too old - Invariant TSC not supported\n");
            }
        } else {
            panic!("Processor too old - x86_cpuid_max_extended_leaf:{:X}\n",
                x86_cpuid_max_extended_leaf()
            );
        }
    }
    pub fn read(&self) -> u64 {
        let (upper, lower): (u64, u64);
        unsafe {
            asm!("rdtsc", out("rdx")upper, out("rax")lower);
        }
        (upper << 32) | lower
    }
    pub fn freq_hz(&self) -> u64 {
        self.freq_hz
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

mod x86_pit {
    #![allow(dead_code)]
    use crate::arch::{x86_ioport_read, x86_ioport_write};

    // I/O Ports
    const PIT_PORT_CH0: u16 = 0x40;
    const PIT_PORT_CH1: u16 = 0x41;
    const PIT_PORT_CH2: u16 = 0x42;
    const PIT_PORT_CMD: u16 = 0x43;
    // Command Fields
    const PIT_ACCESS_LOW_BYTE: u8 = 0x1;
    const PIT_ACCESS_HI_BYTE : u8 = 0x2;
    const PIT_ACCESS_LOW_HI  : u8 = PIT_ACCESS_LOW_BYTE | PIT_ACCESS_HI_BYTE;
    const PIT_OPMODE_ONESHOT : u8 = 0x1;
    const PIT_OPMODE_RATEGEN : u8 = 0x2;
    // Other constants
    const PIT_FREQ_HZ:  u32 = 1193182;
    const PIT_FREQ_KHZ: f64 = 1193.182;

    fn make_cmd(channel: u8, access: u8, opmode: u8) -> u8 {
        ((channel & 0x3) << 6 ) | 
        ((access  & 0x3) << 4 ) |
        ((opmode  & 0x7) << 1 )
    }

    pub fn config_periodic_irq(hz: u16) {
        let reload : u16 = (PIT_FREQ_HZ / hz as u32) as u16;
        let cmd = make_cmd(0, PIT_ACCESS_LOW_HI, PIT_OPMODE_RATEGEN);
        x86_ioport_write::<u8>(PIT_PORT_CMD, cmd);
        x86_ioport_write::<u8>(PIT_PORT_CH0, (reload & 0xFF) as u8);
        x86_ioport_write::<u8>(PIT_PORT_CH0, (reload >> 8)   as u8);
    }

    pub fn config_oneshot_count(ms: u32) {
        // Use Channel 2 in One-Shot mode to count down to zero
        
        // Since CH2 is wired to the PC speaker, the gated-output of the speaker
        // should be disabled first (see bits 0-1 of port 0x61, SysControlPortB)
        let sys_ctl_b: u8 = x86_ioport_read::<u8>(0x61);
        x86_ioport_write::<u8>(0x61, sys_ctl_b & 0xFC);
        // 
        let reload : u16 = (PIT_FREQ_KHZ * ms as f64) as u16;
        let cmd = make_cmd(2, PIT_ACCESS_LOW_HI, PIT_OPMODE_ONESHOT);
        x86_ioport_write::<u8>(PIT_PORT_CMD, cmd);
        x86_ioport_write::<u8>(PIT_PORT_CH0, (reload & 0xFF) as u8);
        x86_ioport_write::<u8>(PIT_PORT_CH0, (reload >> 8)   as u8);
    }

    // Should be called after config_oneshot_sleep is called
    pub fn start_oneshot_count(){
        // Clear and then reset bit 0 of IO port 0x61, after modifying the
        // reload value, hence, start counting down.
        let sys_ctl_b: u8 = x86_ioport_read::<u8>(0x61);
        x86_ioport_write::<u8>(0x61, sys_ctl_b & 0xFE); // Clear bit 0
        x86_ioport_write::<u8>(0x61, sys_ctl_b | 0x01); // Set bit 0
    }

    pub fn wait_for_oneshot_count() {
        // bit 5 of port 0x61 will go high once the counter hits zero
        while x86_ioport_read::<u8>(0x61) & 0x20 == 0 {
        }
    }
}

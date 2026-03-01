// 
// BlightOS Kernel
// 
// Support module for the AARCH64 architecture
//
#![allow(dead_code)]

mod fdt;
mod systimer;
mod mmu;
use crate::arch::*;
use core::sync::atomic::*;
use core::arch::asm;
use fdt::FdtMachineResources;
use crate::sched::Task;
use crate::mem::virt::AddressSpace;
use crate::{SyscallHandlerFn, SyscallOpCode};
use crate::drivers::video::framebuffer::*;
use crate::util::*;

// Re-export the following modules under crate::arch::
pub use self::systimer::*;
pub use self::mmu::*;

#[cfg(feature="debug_arch")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[AARCH64] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}
#[cfg(not(feature="debug_arch"))]
macro_rules! dbg{
    ($($arg:tt)*) => { };
}

static ARCH_BSP_LOADED: AtomicBool = AtomicBool::new(false);
static CPU_COUNT:       AtomicUsize= AtomicUsize::new(1);
static mut DEV_TREE:    FdtMachineResources = FdtMachineResources::new();
static IOINT_CTRL:      Spinlock<BCM2836IntCtl> = Spinlock::new(BCM2836IntCtl::new());
pub static L1INT_CTRL:  Spinlock<BCM2836L1IntCtl> = Spinlock::new(BCM2836L1IntCtl::new());

pub static VIDEO_CORE:  Spinlock<BCM2835VideoCore> = Spinlock::new(BCM2835VideoCore::new());

percpu_global!{
    pub MMU_ON: AtomicBool = AtomicBool::new(false);
}

#[unsafe(no_mangle)]
extern "C"
fn rust_aarch64_entry_bsp(dtbp: usize, cpuid: usize, 
                    stack_base: usize, _rsvd: usize, kern_entry: usize) {
    if cpuid != 0 {
        rust_aarch64_entry_ap(cpuid, stack_base);
        return;
    }

    // Initialize the PerCPU sections of every processor (default memory values)
    percpu_init_sections();
    // Initialize the PerCPU section of this processor (PerCPU register prep)
    percpu_init_cpu(cpuid);

    kearly_console::init();
    dbg!("rust_aarch64_entry_ap: dtbp={:X} kern_entry={:X}, stack_base={:X}, \
        CurrentEL={:X}\n", dtbp, kern_entry, stack_base, aarch64_cur_exc_lvl());

    //
    // Walk the device tree to find the following:
    // - Secondary CPUs and their spin table.
    // - System memory map
    if dtbp == 0 {
        panic!("Cannot load the kernel without a device tree!");
    }
    
    let dev_tree; // immutable reference to avoid spinlock garbage early on
    unsafe {
        if fdt::fdt_parse_tree(dtbp, &mut DEV_TREE) == false {
            panic!("Cannot load the kernel without a device tree!");
        }
        dev_tree = &DEV_TREE;
    }
    

    // Start the secondary CPUs - Assuming spin-table for the method
    for i in 0..dev_tree.cpu_count as usize {
        unsafe {
            (DEV_TREE.cpus[i].release_addr as *mut usize).write(kern_entry);
        }
    }
    
    // Iterate over the detected devices and start the early drivers:
    // - Interrupt Controller
    // - VideCore
    let mut found_int_ctl_dev = false;
    dbg!("Detected devices:\n");
    for i in 0..dev_tree.device_count as usize {
        dbg!("  {:X?}\n", dev_tree.devices[i]);
        if dev_tree.devices[i].dev_type == fdt::FdtDeviceType::IntCtrl {
            if dev_tree.devices[i].compat.contains("bcm2836-armctrl-ic") {
                IOINT_CTRL.lock().init(dev_tree.devices[i].mmio_base);
                found_int_ctl_dev = true;
            } else if dev_tree.devices[i].compat.contains("bcm2836-l1-intc") {
                L1INT_CTRL.lock().init(dev_tree.devices[i].mmio_base);
                found_int_ctl_dev = true;
            }
        } else if dev_tree.devices[i].dev_type == fdt::FdtDeviceType::VideoCore {
            if dev_tree.devices[i].compat.contains("bcm2835-vec") {
                VIDEO_CORE.lock().init(dev_tree.devices[i].mmio_base, &dev_tree);
            }
        }
    }
    if !found_int_ctl_dev {
        panic!("Didn't find a compatible interrupt controller device");
    }

    // Initialize the MMU for the whole system
    MMUMapping::global_init();

    // End of BSP' initialization - Unleash the secondary processors
    ARCH_BSP_LOADED.store(true, Ordering::Relaxed);

    // Initialize the MMU on this CPU
    MMUMapping::percpu_init();
    MMU_ON.borrow_mut().store(true, Ordering::Relaxed);
    // Start the kernel without a Ramdisk
    crate::kstart(0,  Some(&dev_tree.mmap[0..dev_tree.mmap_count as usize]));
    panic!("SHOULDN'T HAVE REACHED HERE!");
}

fn rust_aarch64_entry_ap(cpuid: usize, _stack_base: usize) {
    CPU_COUNT.fetch_add(1, Ordering::Relaxed);
    while ARCH_BSP_LOADED.load(Ordering::Relaxed) != true {
        core::hint::spin_loop();
    }
    // Initialize the per-cpu segment of this CPU
    percpu_init_cpu(cpuid);

    // Initialize the MMU on this CPU
    MMUMapping::percpu_init();

    dbg!("rust_aarch64_entry_ap: CPU{}, stack_base={:X}, CurrentEL={:X}\n",
        cpuid, _stack_base, aarch64_cur_exc_lvl());
    
    crate::kstart(cpuid as usize, None);
    panic!("SHOULDN'T HAVE REACHED HERE!");
}


// Checks if the string located at of null-terminated (c-style strings)
fn addr_starts_with_str(addr: usize, ref_str: &str) -> bool {
    let mut u8p = addr as *const u8;
    let ref_bytes = ref_str.as_bytes();
    for i in 0..ref_str.len() {
        unsafe {
            if u8p.read() != ref_bytes[i] {
                return false;
            }
            u8p = u8p.add(1);
        }
    }
    true
}

pub struct BCM2835MailBox {
    mmio_base:      usize, // Compat device: bcm2835-mbox
    mbox_start:     usize, // Index of the starting item in mbox that's aligned
    mbox:           [u32; 52] // a 36-item array aligned at a 16-byte boundary
}
impl BCM2835MailBox {
    const   REG_READ:           usize = 0x0;
    const   REG_POLL:           usize = 0x10;
    const   REG_SENDER:         usize = 0x14;
    const   REG_STATUS:         usize = 0x18;
    const   REG_CONFIG:         usize = 0x1C;
    const   REG_WRITE:          usize = 0x20;
    const   STATUS_MBOX_FULL:   u32 = 0x80000000;
    const   STATUS_MBOX_EMPTY:  u32 = 0x40000000;

    const   TAG_RESPONSE:       u32 = 0x80000000;
    pub const fn new() -> Self {
        Self {
            mmio_base:  0,
            mbox:       [0 as u32; 52],
            mbox_start: 0
        }
    }

    pub fn init(&mut self, mmio_addr: usize) {
        self.mmio_base = mmio_addr;
        // find the proper alignment:
        for i in 0..15 {
            if (&self.mbox[i] as *const u32 as usize) % 16 == 0 {
                self.mbox_start = i;
                break;
            }
        }
        // dbg!("mbox_buffer @ {:p}, start index: {} -> @{:p}\n",
        //     &self.mbox[0] as *const u32, self.mbox_start,
        //     &self.mbox[self.mbox_start] as *const u32);
    }

    fn read_register(&self, reg: usize) -> u32 {
        if self.mmio_base == 0 {
            return 0;
        }
        unsafe{
            return ((self.mmio_base + reg) as *const u32).read_volatile();
        }
    }

    fn write_register(&mut self, reg: usize, val: u32) {
        if self.mmio_base == 0 {
            return;
        }
        unsafe{
            ((self.mmio_base + reg) as *mut u32).write_volatile(val);
        }
    }

    pub fn call(&mut self, ch: u8, msg: &mut [u32]) -> bool {
        // Copy the message to our mbox
        for i in 0..36 {
            self.mbox[self.mbox_start + i] = msg[i];
        }
        
        // Wait until the mbox is ready
        while self.read_register(Self::REG_STATUS) & Self::STATUS_MBOX_FULL > 0{
            core::hint::spin_loop();
        }
        // Send the message
        let msg_addr = &self.mbox[self.mbox_start] as *const u32 as u32;
        let cmd = msg_addr | (ch as u32 & 0xF);
        self.write_register(Self::REG_WRITE, cmd);

        /* Poll for the response */
        loop {
            /* is there a response? */
            while self.read_register(Self::REG_STATUS) & Self::STATUS_MBOX_EMPTY > 0 {
                core::hint::spin_loop();
            }
            /* is it a response to our message? */
            if self.read_register(Self::REG_READ) == cmd {
                /* is it a valid successful response? */
                if self.mbox[self.mbox_start + 1]==Self::TAG_RESPONSE {
                    // Copy the response out
                    for i in 0..36 {
                        msg[i] = self.mbox[self.mbox_start + i];
                    }
                    return true;
                }
            }
        }
    }
} 



pub struct BCM2835VideoCore {
    mmio_base:      usize,  // Compat device: bcm2835-vec
    mbox:           BCM2835MailBox,
    pub enabled:    bool,
}
impl BCM2835VideoCore {

    const MBOX_REQUEST:     u32 = 0;
    /* channels */
    // const MBOX_CH_POWER:    u32 = 0;
    // const MBOX_CH_FB:       u32 = 1;
    // const MBOX_CH_VUART:    u32 = 2;
    // const MBOX_CH_VCHIQ   3
    // const MBOX_CH_LEDS    4
    // const MBOX_CH_BTNS    5
    // const MBOX_CH_TOUCH   6
    // const MBOX_CH_COUNT   7
    const MBOX_CH_PROP :    u8 = 8;
    /* tags */
    // const MBOX_TAG_SETPOWER       0x28001
    // const MBOX_TAG_SETCLKRATE     0x38002
    const MBOX_TAG_LAST:    u32 = 0;

    pub const fn new() -> Self {
        Self {
            mmio_base:  0,
            mbox:       BCM2835MailBox::new(),
            enabled:    false,
        }
    }

    pub fn init(&mut self, mmio_addr: usize, dt: &FdtMachineResources) {
        self.mmio_base = mmio_addr;
        // Gotta find a compatible MailBox
        for i in 0..dt.device_count as usize {
            if dt.devices[i].dev_type == fdt::FdtDeviceType::MailBox &&
                dt.devices[i].compat.contains("bcm2835-mbox") {
                self.mbox.init(dt.devices[i].mmio_base);
                self.enabled = true;
                break;
            }
        }
        if !self.enabled {
            return;
        }
        // Set the initial FB
        let mut msg: [u32; 36] = [0; 36];
        msg[0]  = 35*4;
        msg[1]  = Self::MBOX_REQUEST;

        msg[2] = 0x48003;  //set phy wh
        msg[3] = 8;
        msg[4] = 8;
        msg[5] = 1920;         //FrameBufferInfo.width
        msg[6] = 1080;          //FrameBufferInfo.height

        msg[7] = 0x48004;  //set virt wh
        msg[8] = 8;
        msg[9] = 8;
        msg[10] = 1920;        //FrameBufferInfo.virtual_width
        msg[11] = 1080;         //FrameBufferInfo.virtual_height

        msg[12] = 0x48009; //set virt offset
        msg[13] = 8;
        msg[14] = 8;
        msg[15] = 0;           //FrameBufferInfo.x_offset
        msg[16] = 0;           //FrameBufferInfo.y.offset

        msg[17] = 0x48005; //set depth
        msg[18] = 4;
        msg[19] = 4;
        msg[20] = 32;          //FrameBufferInfo.depth

        msg[21] = 0x48006; //set pixel order
        msg[22] = 4;
        msg[23] = 4;
        msg[24] = 0;           //RGB, not BGR preferably

        msg[25] = 0x40001; //get framebuffer, gets alignment on request
        msg[26] = 8;
        msg[27] = 8;
        msg[28] = 4096;        //FrameBufferInfo.pointer
        msg[29] = 0;           //FrameBufferInfo.size

        msg[30] = 0x40008; //get pitch
        msg[31] = 4;
        msg[32] = 4;
        msg[33] = 0;           //FrameBufferInfo.pitch

        msg[34] = Self::MBOX_TAG_LAST;

        //this might not return exactly what we asked for, could be
        //the closest supported resolution instead
        if self.mbox.call(Self::MBOX_CH_PROP, &mut msg) &&
                msg[20]==32 && msg[28]!=0
        {
            let mut fb = FrameBuffer::new();
            fb.width=msg[5];          //get actual physical width
            fb.height=msg[6];         //get actual physical height
            fb.pitch=msg[33];         //get number of bytes per line
            fb.bpp = msg[20] as u8;   // depth
            // TODO - proper bus2mem address translation from the device tree
            fb.base_address = (msg[28] & 0x3FFFFFFF) as usize; 
            // dbg!("FB initialized @ {:X} {}x{} ({} bpp)\n",
            //     self.fb.base_address, self.fb.width, self.fb.height,
            //     self.fb.bpp
            // );
            fb.background_rgb = (200, 200, 200);
            fb.foreground_rgb = (0, 0, 220);
            FrameBuffer::register(&fb);
            FrameBuffer::clean_screen();
        } else {
            dbg!("Unable to set screen resolution to 1024x768x32\n");
        }
    }

}

pub struct BCM2836L1IntCtl {
    // Compat: bcm2836-l1-intc 
    // per-cpu interrupt controller for the timer, PMU events, and SMP IPIs.
    pub mmio_base:      usize,
}
impl BCM2836L1IntCtl {
    const REG_CORE_TIMER_INT_CTL:   usize = 0x40;
    const REG_CORE_MAILBOX_INT_CTL: usize = 0x50;
    const REG_CORE_IRQ_SRC:         usize = 0x60;
    const REG_CORE_FIQ_SRC:         usize = 0x70;
    pub const fn new() -> Self {
        Self {
            mmio_base:  0,
        }
    }

    pub fn init(&mut self, mmio_addr: usize) {
        self.mmio_base = mmio_addr;
    }

    pub fn mask_core_timer_irq(irq_masked: bool) {  
        let reg = Self::REG_CORE_TIMER_INT_CTL + cpu_id() * 4;
        if irq_masked {
            L1INT_CTRL.lock().write_register(reg, 0);
        } else {
            L1INT_CTRL.lock().write_register(reg, 0x2); // use fiq
        }
    }

    pub fn core_timer_irq_status() -> bool {
        let reg = Self::REG_CORE_IRQ_SRC + cpu_id() * 4;
        L1INT_CTRL.lock().read_register(reg) & 2 > 0
    }

    fn read_register(&self, reg: usize) -> u32 {
        if self.mmio_base > 0 {
            // klog!("reading from {:X}...\n", self.mmio_base + reg);
            unsafe{
                return ((self.mmio_base + reg) as *const u32).read_volatile();
            }
        }
        0
    }

    fn write_register(&mut self, reg: usize, val: u32) {
        // klog!("writing to {:X} to {:X}...\n", val, self.mmio_base + reg);
        if self.mmio_base > 0 {
            unsafe{
                ((self.mmio_base + reg) as *mut u32).write_volatile(val);
            }
        }
    }
}

pub struct BCM2836IntCtl {
    // compat: bcm2836-armctrl-ic
    // Peripherals Interrupt Controller
    pub mmio_base:      usize
}
impl BCM2836IntCtl {
    const REG_IRQ_BASIC_PENDING:    usize = 0x00;
    const REG_IRQ_PENDING1:         usize = 0x04;
    const REG_IRQ_PENDING2:         usize = 0x08;
    const REG_IRQ_FIQ_CONTROL:      usize = 0x0C;
    const REG_IRQ_ENABLE1:          usize = 0x10;
    const REG_IRQ_ENABLE2:          usize = 0x14;
    const REG_IRQ_BASIC_ENABLE:     usize = 0x18;
    const REG_IRQ_DISABLE1:         usize = 0x1C;
    const REG_IRQ_DISABLE2:         usize = 0x20;
    const REG_IRQ_BASIC_DISABLE:    usize = 0x24;
    pub const fn new() -> Self {
        Self {
            mmio_base:  0
        }
    }

    pub fn init(&mut self, base_addr: usize) {
        self.mmio_base = base_addr;
    }

    fn read_register(&self, reg: usize) -> u32 {
        if self.mmio_base > 0 {
            klog!("reading from {:X}...\n", self.mmio_base + reg);
            unsafe{
                return ((self.mmio_base + reg) as *const u32).read_volatile();
            }
        }
        0
    }

    fn write_register(&mut self, reg: usize, val: u32) {
        klog!("writing to {:X} to {:X}...\n", val, self.mmio_base + reg);
        if self.mmio_base > 0 {
            unsafe{
                ((self.mmio_base + reg) as *mut u32).write_volatile(val);
            }
        }
    }

    pub fn pending_irqs(&self) -> (u32, u32, u32) {
        (self.read_register(0x60),
            self.read_register(Self::REG_IRQ_PENDING1),
            self.read_register(Self::REG_IRQ_PENDING2))
    } 
}

pub mod bcm_peripherals {
    use crate::arch::*;
    use core::sync::atomic::*;
    // TODO - Shouldn't hardcode these once DT enumeration is ported
    // RPi 3A+ and 3B+
    const BCM_PERIPHERAL_BASE:      usize = 0x3F000000;
    // RPi 4
    // const BCM_PERIPHERAL_BASE:      usize = 0xFE000000;
    const BCM_AUX_BASE:             usize = BCM_PERIPHERAL_BASE + 0x215000;

    #[repr(usize)]
    enum BCMRegister{
        // GPIO Registers
        BcmGpioBase     = BCM_PERIPHERAL_BASE + 0x200000,
        GPFSel1         = Self::BcmGpioBase as usize + 0x04,
        GPSet0          = Self::BcmGpioBase as usize + 0x1C,
        GPClr0          = Self::BcmGpioBase as usize + 0x28,
        GPPUD           = Self::BcmGpioBase as usize + 0x94,
        GPPUDClk0       = Self::BcmGpioBase as usize + 0x98,

        // PL011-UART
        Uart0DR         = Self::BcmGpioBase as usize + 0x1000, // UART0_BASE
        Uart0RSRECR     = Self::Uart0DR as usize + 0x04,
        Uart0FR         = Self::Uart0DR as usize + 0x18,
        Uart0ILPR       = Self::Uart0DR as usize + 0x20,
        Uart0IBRD       = Self::Uart0DR as usize + 0x24,
        Uart0FBRD       = Self::Uart0DR as usize + 0x28,
        Uart0LCRH       = Self::Uart0DR as usize + 0x2C,
        Uart0CR         = Self::Uart0DR as usize + 0x30,
        Uart0IFLS       = Self::Uart0DR as usize + 0x34,
        Uart0IMSC       = Self::Uart0DR as usize + 0x38,
        Uart0RIS        = Self::Uart0DR as usize + 0x3C,
        Uart0MIS        = Self::Uart0DR as usize + 0x40,
        Uart0ICR        = Self::Uart0DR as usize + 0x44,
        Uart0DMACR      = Self::Uart0DR as usize + 0x48,
        Uart0ITCR       = Self::Uart0DR as usize + 0x80,
        Uart0ITIP       = Self::Uart0DR as usize + 0x84,
        Uart0ITOP       = Self::Uart0DR as usize + 0x88,
        Uart0TDR        = Self::Uart0DR as usize + 0x8C,

        // minUART
        BcmAuxBase      = BCM_PERIPHERAL_BASE + 0x215000,
        AuxEnables      = Self::BcmAuxBase as usize + 0x04,
        AuxMuIO         = Self::BcmAuxBase as usize + 0x40,
        AuxMuIER        = Self::BcmAuxBase as usize + 0x44,
        AuxMuIIR        = Self::BcmAuxBase as usize + 0x48,
        AuxMuLCR        = Self::BcmAuxBase as usize + 0x4C,
        AuxMuMCR        = Self::BcmAuxBase as usize + 0x50,
        AuxMuLSR        = Self::BcmAuxBase as usize + 0x54,
        AuxMuMSR        = Self::BcmAuxBase as usize + 0x58,
        AuxMuScratch    = Self::BcmAuxBase as usize + 0x5C,
        AuxMuCntl       = Self::BcmAuxBase as usize + 0x60,
        AuxMuStat       = Self::BcmAuxBase as usize + 0x64,
        AuxMuBaud       = Self::BcmAuxBase as usize + 0x68,

    }

    fn read_register(reg: BCMRegister) -> u32 {
        if crate::arch::MMU_ON.borrow().load(Ordering::Relaxed) {
            unsafe {
                (MMUMapping::dma_from_kernel_phys(reg as usize) as *const u32).
                        read_volatile()
            }
        } else {
            unsafe {
                (reg as usize as *const u32).read_volatile()
            }
            
        }
        
    }

    fn write_register(reg: BCMRegister, val: u32) {
        if crate::arch::MMU_ON.borrow().load(Ordering::Relaxed) {
            unsafe {
                (MMUMapping::dma_from_kernel_phys(reg as usize) as *mut u32).
                        write_volatile(val);
            }
        } else {
            unsafe {
                (reg as usize as *mut u32).write_volatile(val);
            }
        }
    }
    ////////////////////////////////////////////

    pub fn miniuart_init() {
        write_register(BCMRegister::AuxEnables, 1);
        write_register(BCMRegister::AuxMuIER,   0);
        write_register(BCMRegister::AuxMuCntl,  0);
        write_register(BCMRegister::AuxMuLCR,   3);
        write_register(BCMRegister::AuxMuMCR,   0);
        write_register(BCMRegister::AuxMuIIR,0xC6);
        write_register(BCMRegister::AuxMuBaud,270);

        let mut ra=read_register(BCMRegister::GPFSel1);
        ra &= !(7 << 12);   //gpio14
        ra |= 2<<12;        //alt5
        write_register(BCMRegister::GPFSel1, ra);

        write_register(BCMRegister::GPPUD, 0);
        for _i in 0..150 {
            core::hint::spin_loop();
        }
        write_register(BCMRegister::GPPUDClk0, (1<<14)|(1<<15));
        for _i in 0..150 {
            core::hint::spin_loop();
        }
        write_register(BCMRegister::GPPUDClk0,0);

        // Specific to RPI4 PLAT_BCM2711
		// System clock freq = VPU Clock = 500MHz
        // baud = (system_clock_freq)/(8(baud_reg+1))
        // write_register(AUX_MU_BAUD_REG,541); //115200

        write_register(BCMRegister::AuxMuCntl,2);    
    }

    fn miniuart_putc(c: u8) {
        loop {
            if read_register(BCMRegister::AuxMuLSR) & 0x20 > 0 {
                break;
            }
        }
        write_register(BCMRegister::AuxMuIO, c as u32);
    }

    pub fn miniuart_print_str(msg: &[u8]) {
        for c in msg {
            miniuart_putc(*c);
        }
    }

    //////////////
    pub fn pl011uart_init() {
        // Disable UART0.
	    write_register(BCMRegister::Uart0CR, 0x00000000);
	    // Setup the GPIO pin 14 && 15.
 
	    // Disable pull up/down for all GPIO pins & delay for 150 cycles.
	    write_register(BCMRegister::GPPUD, 0x00000000);
	    for _i in 0..150 {
            core::hint::spin_loop();
        }
 
	    // Disable pull up/down for pin 14,15 & delay for 150 cycles.
	    write_register(BCMRegister::GPPUDClk0, (1 << 14) | (1 << 15));
	    for _i in 0..150 {
            core::hint::spin_loop();
        }
 
	    // Write 0 to GPPUDCLK0 to make it take effect.
	    write_register(BCMRegister::GPPUDClk0, 0x00000000);
 
	    // Clear pending interrupts.
	    write_register(BCMRegister::Uart0ICR, 0x7FF);
 
	    // Set integer & fractional part of baud rate.
	    // Divider = UART_CLOCK/(16 * Baud)
	    // Fraction part register = (Fractional part * 64) + 0.5
	    // Baud = 115200.
 
	    // TODO: FIX THIS
        // For Raspi3 and 4 the UART_CLOCK is system-clock dependent by default.
	    // Set it to 3Mhz so that we can consistently set the baud rate
	    // if (raspi >= 3) {
	    // 	// UART_CLOCK = 30000000;
	    // 	unsigned int r = (((unsigned int)(&mbox) & ~0xF) | 8);
	    // 	// wait until we can talk to the VC
	    // 	while ( mmio_read(MBOX_STATUS) & 0x80000000 ) { }
	    // 	// send our message to property channel and wait for the response
	    // 	mmio_write(MBOX_WRITE, r);
	    // 	while ( (mmio_read(MBOX_STATUS) & 0x40000000) || mmio_read(MBOX_READ) != r ) { }
	    // }
 
	    // Divider = 3000000 / (16 * 115200) = 1.627 = ~1.
	    write_register(BCMRegister::Uart0IBRD, 1);
	    // Fractional part register = (.627 * 64) + 0.5 = 40.6 = ~40.
	    write_register(BCMRegister::Uart0FBRD, 40);
 
	    // Enable FIFO & 8 bit data transmission (1 stop bit, no parity).
	    write_register(BCMRegister::Uart0LCRH, (1 << 4) | (1 << 5) | (1 << 6));
 
	    // Mask all interrupts.
	    write_register(BCMRegister::Uart0IMSC, (1 << 1) | (1 << 4) | (1 << 5) | (1 << 6) |
	                       (1 << 7) | (1 << 8) | (1 << 9) | (1 << 10));
 
	    // Enable UART0, receive & transfer part of UART.
	    write_register(BCMRegister::Uart0CR, (1 << 0) | (1 << 8) | (1 << 9));
    }

    pub fn pl011uart_getc() -> Option<u8> {
        if read_register(BCMRegister::Uart0FR) & 0x10 == 0 {
            return Some(read_register(BCMRegister::Uart0DR) as u8);
        }
        None
    }

    fn pl011uart_putc(c: u8){
	    // Wait for UART to become ready to transmit.
	    loop {
            if read_register(BCMRegister::Uart0FR) & 0x20 == 0 {
                break;
            }
        }
	    write_register(BCMRegister::Uart0DR, c as u32);
    }

    pub fn pl011uart_print_str(msg: &[u8]) {
        for c in msg {
            pl011uart_putc(*c);
        }
    }
}

pub mod kearly_console {
    use crate::drivers::video::framebuffer::*;
    use crate::util::*;

    static CURSOR : Spinlock<(u32, u32)> = Spinlock::new((0,0));

    pub fn init() {
        crate::arch::bcm_peripherals::pl011uart_init();
        if FrameBuffer::enabled() {
            let mut cursor = CURSOR.lock();
            cursor.0 = 0;
            cursor.1 = 0;
            FrameBuffer::clean_screen();
        }
    }

    pub fn print_str(msg: &[u8]) {
        if FrameBuffer::enabled() {
            print_str_vga(msg);
        } else {
            crate::arch::bcm_peripherals::pl011uart_print_str(msg);
        }

    }

    fn print_str_vga(msg: &[u8]) {
        let (sh, sw) = FrameBuffer::screen_size();
        let (fh, fw) = FrameBuffer::font_size();
        let (rows, cols) = (sh / fh, sw / fw);
        let mut cursor = CURSOR.lock();
        for &c in msg {
            if c == b'\n' {
                (*cursor).0 = ((*cursor).0 + 1) % rows;
                (*cursor).1 = 0;
            } else if c == 0x8 { // Backspace
                if (*cursor).1 > 0 {
                    (*cursor).1  -= 1;
                    FrameBuffer::putc(b' ', (*cursor).0, (*cursor).1);
                }
            } else {
                FrameBuffer::putc(c, (*cursor).0, (*cursor).1);
                (*cursor).1 = (*cursor).1 + 1;
                if (*cursor).1 == cols {
                    (*cursor).1 = 0;
                    (*cursor).0 = ((*cursor).0 + 1) % rows;
                }
            }
        }
    }
}

pub mod kdebug_console {
    
    pub fn init() {
        crate::arch::bcm_peripherals::pl011uart_init();
    }

    pub fn print_str(msg: &[u8]) {
        crate::arch::bcm_peripherals::pl011uart_print_str(msg);
    }
}

pub fn aarch64_cur_exc_lvl() -> usize {
    let cur_el: u64;
    unsafe {
        asm!(
            "mrs {0}, CurrentEL",
            out(reg)cur_el
        );
    }
    (cur_el >> 2) as usize
}

pub fn cpu_busywait(delay: Duration) {
    let target_tsc = SystemTimer::current_timestamp() + 
                        SystemTimer::duration_to_timestamp_ticks(delay);
    while SystemTimer::current_timestamp() < target_tsc {
        core::hint::spin_loop();
    }
}

pub fn cpu_count() -> usize {
    CPU_COUNT.load(Ordering::Relaxed)
}


pub fn cpu_id() -> usize {
    let mpidr_el1: usize;
    unsafe {
        asm!(
            "mrs {0}, mpidr_el1",
            out(reg)mpidr_el1
        );
    }
    mpidr_el1 & 0xFF
}

pub fn cpu_halt() {
    unsafe { asm!("wfe"); }
}

pub fn cpu_ints_enabled() -> bool {
    let daif: usize;
    unsafe {
        asm!(
            "mrs {0}, daif",
            out(reg)daif
        )
    }
    (daif & 0x3c0) == 0
}
pub fn cpu_enable_ints() {
    aarch64_spsr_el1_unmask_ints(); // To make sure ints stay on after eret
    unsafe { asm!( "msr DAIFClr, #0b1111"); }
}
pub fn cpu_disable_ints() {
    unsafe { asm!( "msr DAIFSet, #0b1111"); }
}

pub fn cpu_restore_ints(int_en: bool) {
    if int_en {
        cpu_enable_ints();
    } else {
        cpu_disable_ints();
    }
}

pub fn aarch64_ttbr1_el1() -> usize {
    let ttbr1_el1: usize;
    unsafe {
        asm!(
            "mrs {0}, ttbr1_el1",
            out(reg)ttbr1_el1
        )
    }
    ttbr1_el1
}

//
// PerCpu Storage Support - util::PerCpuGlobal<T> requires the following
// architecture-dependent functions to be defined here:
// percpu_borrow and percpu_borrow_mut
// The rest of the kernel code should not use these interfaces as they are not
// thread- or type-safe
//
// TPIDR_EL1: Software Thread ID Register is used (similar to GS for x64) as
//            the processor doesn't use it at all
//
fn percpu_init_sections() {
    // Copy the first percpu section into the subsequent N-1 sections
    // (see link.ld)
    unsafe extern "C" {
        static _KERNEL_PERCPU_START:usize; 
        static _KERNEL_PERCPU_SIZE: usize;
        static _KERNEL_PERCPU_END:  usize;
    }
    unsafe {
        let sect_size: usize = &_KERNEL_PERCPU_SIZE as *const usize as usize;
        let pcpu_s:usize = &_KERNEL_PERCPU_START as *const usize as usize;
        let pcpu_e:usize = &_KERNEL_PERCPU_END as *const usize as usize;
        if sect_size < 1 {
            dbg!("NO PERCPU VARIABLE!\n");
            return;
        }
        let nsects  = (pcpu_e - pcpu_s) / sect_size;
        dbg!("PERCPU: VMA {:X} bytes starting @ 0, LMA[{:X} - {:X}], #Copies: {}\n",
            sect_size, pcpu_s, pcpu_e, nsects);
        for s in 1..nsects {
            let base = pcpu_s + s * sect_size;
            (base as *mut u8).copy_from_nonoverlapping(
                                pcpu_s as *const u8, sect_size);
        }
    }
}
fn percpu_init_cpu(cpuid: usize) {
    // 1) Store the base address of the corresponding PerCPU section in %gs
    // 2) Set the THIS_PERCPU_BASE variable to the absolute address of the
    //    percpu segement for future use. Reading from %gs:off is faster than
    //    reading the MSR itself.
    unsafe extern "C" {
        static _KERNEL_PERCPU_START:usize; 
        static _KERNEL_PERCPU_SIZE: usize;
    }
    unsafe {
        let pcpu_s:    usize = &_KERNEL_PERCPU_START as *const usize as usize;
        let sect_size: usize = &_KERNEL_PERCPU_SIZE as *const usize as usize;
        let base_addr = pcpu_s + cpuid * sect_size;
        asm!(
            "msr TPIDR_EL1, {0}",
            in(reg)base_addr
        );
    }
}
pub fn percpu_borrow<T>(var: &T) -> &T {
    unsafe {
        let mut addr : usize;
        asm!("mrs {0}, TPIDR_EL1", out(reg)addr);
        addr = addr + var as *const T as usize;
        &(*(addr as *mut T))
    }
}
pub fn percpu_borrow_mut<T>(var: &T) -> &mut T {
    unsafe {
        let mut addr : usize;
        asm!("mrs {0}, TPIDR_EL1", out(reg)addr);
        addr = addr + var as *const T as usize;
        &mut *(addr as *mut T)
    }
}


pub fn aarch64_stack_pointer() -> usize {
    let stack_pointer: usize;
    unsafe {
        asm!(
            "mov {0}, sp",
            out(reg)stack_pointer
        )
    }
    stack_pointer
}

pub fn aarch64_stack_pointer_el0() -> usize {
    let sp_el0: usize;
    unsafe {
        asm!(
            "mrs {0}, sp_el0",
            out(reg)sp_el0
        )
    }
    sp_el0
}

fn aarch64_spsr_el1() -> usize {
    let spsr_el1: usize;
    unsafe {
        asm!(
            "mrs {0}, spsr_el1",
            out(reg)spsr_el1
        )
    }
    spsr_el1
}

fn aarch64_write_spsr_el1(val: usize) {
    unsafe {
        asm!(
            "msr spsr_el1, {0}",
            "isb sy",
            "dsb sy",
            in(reg)val
        )
    }
}

// ERET uses SPSR to restore the program flags.
// This function ensures DIAF is re-enabled upon ERET
pub fn aarch64_spsr_el1_unmask_ints() {
   aarch64_write_spsr_el1(aarch64_spsr_el1() & !0x3c0);
}

#[unsafe(no_mangle)]
extern "C"
fn aarch64_elx_exception(syndrome: usize, fault_addr: usize) {
    let exception_class = (syndrome & 0xFC000000) >> 26;
    let iss = syndrome & 0x1FFFFFF;
    match exception_class {
        0x15        => { // SVC Instruction
            if iss == 1000 {
                klog!("<SVC>");
            } else if iss == 1001 { // cpu_trigger_systimer_irq was called
                SystemTimer::exec_handler();
            }
        },
        0x25        => {
            klog!("<Data Abort, ELx> FA:{:X}\n", fault_addr);
            panic!("");
        }
        _           => {
            let el0_sp: u64;
            let el1_sp = aarch64_stack_pointer();
            let el1_elr:u64;
            let el1_spsr:u64;
            unsafe {
                asm!(
                    "mrs    {0}, sp_el0",
                    "mrs    {1}, elr_el1",
                    "mrs    {2}, spsr_el1",
                    out(reg)el0_sp,
                    out(reg)el1_elr,
                    out(reg)el1_spsr
                );
            }
            klog!("Unhandled Exception: Synd:{:X} (cls:{:X}), FA:{:X}\n", 
                syndrome, exception_class, fault_addr);
            klog!("TID:{} EL0: sp={:X}, EL1: sp={:X},elr={:X},spsr={:X}\n",
                    Task::current_tid(), el0_sp, el1_sp, el1_elr, el1_spsr);
            // klog!("EL0: sp={:X}    EL1: sp={:X}\n",
            //      el0_sp, el1_sp);
            panic!("aarch64_elx_exception");
        }
    }
}

#[unsafe(no_mangle)]
fn aarch64_elx_irq() {
    // let spsr = aarch64_spsr_el1();
    // aarch64_spsr_el1_unmask_ints();
    if SystemTimer::irq_pending() {
        if cpu_id() == 0 {
            // klog!("<IRQx0-Tick>");
        }
        // Preemption IRQ        
        SystemTimer::exec_handler();
    } else {
        klog!("<IRQ-?>");
    }
}

#[unsafe(no_mangle)]
fn aarch64_elx_fiq() {
//     let spsr = aarch64_spsr_el1();
//     if cpu_id() == 0 {
//         klog!("<FIQ{},{:X}>", cpu_id(), spsr);
//     }
    klog!("<FIQx>");
    if SystemTimer::irq_pending() {
        // Preemption FIQ
        SystemTimer::exec_handler();
    } else {
        // klog!("<FIQ ELx>");
    }
}

#[unsafe(no_mangle)]
extern "C"
fn aarch64_ivt_excep_lower_el(x0: usize, x1: usize, x2: usize, x3: usize,
                        x4: usize) {
    let excep_syndrome: usize;
    let fault_addr: usize;
    let inst_addr: usize;
    unsafe {
        asm!(
            "mrs {0}, ESR_EL1",
            "mrs {1}, FAR_EL1",
            "mrs {2}, ELR_EL1",
            out(reg)excep_syndrome,
            out(reg)fault_addr,
            out(reg)inst_addr
        );
    }
    let exception_class = (excep_syndrome & 0xFC000000) >> 26;
    let iss = excep_syndrome & 0x1FFFFFF;
    
    match exception_class {
        0x15        => { // SVC Instruction
            if iss == 1000 {
                // System call
                ksyscall_handler(x0, x1, x2, x3, x4);
            }else if iss == 1001 {
                // This shouldn't be invoked from EL0 ....
                panic!("SystemTimer::exec_handler() called from EL0!");
            } else {
                klog!("<ELE{:X}>", exception_class);
            }
        },
        0x24        => {
            klog!("<Data Abort, lower> FA:{:X}\n", fault_addr);
            panic!("");
        },
        0x0     => { // Unknown reason
            // Check illegal instruction address for the user-space
            if inst_addr < MMUMapping::MIN_VIRTUAL as usize || 
                inst_addr >= MMUMapping::MAX_VIRTUAL as usize {
                if AddressSpace::handle_page_fault(inst_addr) {
                    return;
                }
                klog!("<Illegal Execution by EL0> ELR_EL1:{:X}\n", inst_addr);
                panic!("");
            } else {
                klog!("<Unknown Exception, lower> FA:{:X}, ELR_EL1:{:X}\n",
                    fault_addr, inst_addr);
                panic!("");
            }
        }
        _           => {
            let ttbr1: u64;
            let ttbr0: u64;
            unsafe {
                asm!(
                    "mrs {0}, ttbr1_el1",
                    "mrs {1}, ttbr0_el1",
                    out(reg)ttbr1,
                    out(reg)ttbr0,
                );
            }
            klog!("Unhandled Exception: Synd:{:X} (cls:{:X}), FA:{:X}, ttbr1:{:X}, \
                    ttbr0:{:X}, elr_el1:{:X}\n", 
                excep_syndrome, exception_class, fault_addr, ttbr1, ttbr0,
                inst_addr);
            panic!("aarch64_ivt_excep_lower_el");
        }
    }
}



#[unsafe(no_mangle)]
fn aarch64_ivt_irq_el0() {
    panic!("aarch64_ivt_irq_el0");
}

#[unsafe(no_mangle)]
fn aarch64_ivt_serr_el0() {
    panic!("aarch64_ivt_serr_el0");
}

#[unsafe(no_mangle)]
fn aarch64_ivt_irq_lower_el(){
    // aarch64_elx_irq();
    if SystemTimer::irq_pending() {
        // Preemption IRQ
        // klog!("<ILE-TICK>");
        SystemTimer::exec_handler();
    } else {
       klog!("<ILE-?>");
    }
}

#[unsafe(no_mangle)]
fn aarch64_ivt_default(){
    klog!("<DEF>");
}

#[unsafe(no_mangle)]
fn aarch64_cs_error(sp: usize) {
    dump_memory(sp, 20);
    panic!("Corrupted Context");
}

//
// SYSCALL Interface
//
fn syscall_default_imp(arg0: usize, arg1: usize, arg2: usize, arg3: usize) {
    klog!("Syscall({:X}, {:X}, {:X}, {:X}) - not registered.",
            arg0, arg1, arg2, arg3);
}
static mut X64_SYSCALL_HANDLER:[SyscallHandlerFn; SyscallOpCode::Max as usize] = 
                            [syscall_default_imp; SyscallOpCode::Max as usize];

pub fn syscall_register(opcode: SyscallOpCode, handler: SyscallHandlerFn) -> bool {
    if opcode < SyscallOpCode::Max {
        unsafe {
            X64_SYSCALL_HANDLER[opcode as usize] = handler;
        }
        return true;
    }
    false
}

pub fn syscall_trigger_int(opcode: usize,
                        arg0: usize, arg1: usize, arg2: usize, arg3: usize) {
    unsafe{
        asm!(
            "svc #1000", // Generates a sync_excep_elx
            in("x0") opcode,
            in("x1") arg0,
            in("x2") arg1,
            in("x3") arg2,
            in("x4") arg3,
        );
    };
}


pub fn cpu_trigger_systimer_irq() {
    unsafe{
        asm!(
            "svc #1001"
        );
    };
}

fn ksyscall_handler(opcode: usize, a0: usize, a1: usize, a2: usize, a3: usize) {
    if opcode < SyscallOpCode::Max as usize {
        unsafe {
            X64_SYSCALL_HANDLER[opcode](a0, a1, a2, a3);
        }
    }
}

//
// Task Management
// + Context creation
// + Context switch
//
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct TaskContext {
    ep:     fn(usize),  // Initial RIP value, i.e., Entry-point
    sp:     usize,      // Last RSP (Stack Pointer) value
    tid:    usize,      // For debugging purposes
}

impl TaskContext {

    pub const fn new() -> Self {
        Self {
            ep:     empty_task,
            sp:     0,
            tid:    0,
        }
    }

    pub fn init(&mut self, id: usize, func: fn(usize), func_arg: usize,
                                                        stack: &mut [usize]) {
        let stacklen = stack.len();
        // Initial stack - compatible with the context switch logic in boot.S
        // ----- stack_base -------------------------------
        // ....
        // RSP -> 0x0123456789abcdef "Stack watermark" <-- 800 byte from base
        //        0x0123456789abcdef "Stack watermark"
        //        spsr_el1
        //        elr_el1
        //        ...... Initial register values
        //        x0
        // ---- stack_base + stack_size ------------------- <- &stack[stacklen]
        stack[stacklen - 1]  = func_arg; // x1: argument for the task's function
        stack[stacklen - 2]  = (self as *const TaskContext) as usize; // x0
        // The usual save/switch/restore path is activated via SVC or IRQs and
        // the context-switch should use ret to avoid conflicts with the eret
        // from the SVC/IRQ handler => put the launch_task address in the link
        // register, i.e., x30
        stack[stacklen - 32] = Self::launch_task as *const () as usize; // x30 ret
        // The first task on the CPU needs to enable the interrupts upon
        // launch, therefore, it uses an eret (no SVC/IRQ handler called before)
        // and so, lunch_task should also be in ELR_EL1 in case this is the
        // first task on the CPU
        stack[stacklen - 97] = Self::launch_task as *const () as usize; // eret
        stack[stacklen - 98] = 0x5;    // SPSR: EL1h, ints enabled
        stack[stacklen - 99] = 0x1234; // Watermark
        stack[stacklen - 100] = 0x1234; // Watermark

        self.ep = func;
        self.sp = (&stack[stacklen - 100] as *const usize) as usize;
        self.tid = id;
        // let ints = cpu_ints_enabled();
        // klog!("TaskContext::Init(id:{}, fn:{:p}, stack:@0x{:X}, len:{}, \
        //     sp:{:X}, &task: {:X} - Ints?{}\n",
        //     id, func,
        //     (&stack[0] as *const usize) as usize, stacklen * 8, self.sp,
        //     (self as *const TaskContext) as usize, ints
        // );
    }

    // This function is called as a wrapper of the task's callback to handle
    // the return of the task (i.e., exit)
    // User-space tasks have their exit handled differently via the virt module.
    fn launch_task(task: &mut TaskContext, task_arg: usize) {
        // let actual_sp = aarch64_stack_pointer();
        // let inten = cpu_ints_enabled();
        // klog!("Starting task[{}] on CPU {}: @{:X} ep:{:X}, sp:{:X} - \
        //         actual sp:{:X}  - Arg: {:X}, INTS:{:?}\n", 
        //     task.tid, cpu_id(), (task as *const TaskContext) as usize,
        //     task.ep as usize, task.sp, actual_sp, _task_arg, inten);
        cpu_enable_ints();
        (task.ep)(task_arg);
        // Terminate the task
        Task::exit();
        panic!("Continued a dead task's code where it have been unreachable!");
    }
}

fn empty_task(_arg: usize) {
    panic!("This is an empty task. Should never be scheduled!");
}

unsafe extern "C" {
    fn start_first_thread(task_p: usize);
    fn switch_context(old_p: usize,  new_p: usize);
}

// Switch to the context of the specified task without saving the current
// context. Used when the current task is terminating or for the very first
// task before which there is no previous context to retrieve!
pub fn cpu_switch_context_nosave(task: &TaskContext) {
    unsafe{
        start_first_thread(task as *const TaskContext as usize);
    }
}

pub fn cpu_switch_context(from: &TaskContext, to: &TaskContext) {
    // klog!("<CPU {}: TID{} -> TID{}>", cpu_id(), from.tid, to.tid);
    unsafe{
        switch_context(from as *const TaskContext as usize,
                        to as *const TaskContext as usize);
    }
}


pub fn machine_reboot() {
    // TODO 
}
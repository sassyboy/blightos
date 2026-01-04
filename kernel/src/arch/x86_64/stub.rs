// 
// Rust stub for the x86_64 architecture
//
#![allow(dead_code)]
use core::mem::size_of;
use core::arch::asm;
use crate::arch::asc::vga::*;
use crate::pmm::PMMapElement;
use crate::sched;
use crate::util::*;
use crate::{dump_memory, kstart};

mod vga;

//
// Debugging macros
//
use core::fmt::Write;
struct ArchDebugConsole;
impl Write for ArchDebugConsole {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        kearly_console::print_str(_s.as_bytes());
        Ok(())
    }
}

macro_rules! log {
    ($($arg:tt)*) => {
        let mut kern_console = ArchDebugConsole{};
        let _ = write!(&mut kern_console, $($arg)*);
    };
}

#[cfg(feature="debug_arch")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut kern_console = ArchDebugConsole{};
        let _ = write!(&mut kern_console, $($arg)*);
    };
}

#[cfg(not(feature="debug_arch"))]
macro_rules! dbg {
    ($($arg:tt)*) => { };
}

//---------------------------------------------------------------------------//
// Private Data Types and Globals                                            //
//---------------------------------------------------------------------------//
// Multiboot 1 Information
#[repr(C)]
pub struct MultibootInfo {
    flags: u32,
    mem_lower: u32,
    mem_upper: u32,
    boot_device: u32,
    cmdline: u32,
    mods_count: u32,
    mods_addr: u32,
    syms: [u32; 4], // Represents a union in C for a.out or ELF symbols
    mmap_length: u32,
    mmap_addr: u32,
    drives_length: u32,
    drives_addr: u32,
    config_table: u32,
    boot_loader_name: u32,
    apm_table: u32,
    vbe_control_info: u32,
    vbe_mode_info: u32,
    vbe_mode: u16,
    vbe_interface_seg: u16,
    vbe_interface_off: u16,
    vbe_interface_len: u16,
    framebuffer_addr: u64,
    framebuffer_pitch: u32,
    framebuffer_width: u32,
    framebuffer_height: u32,
    framebuffer_bpp: u8,
    framebuffer_type: u8,
    framebuffer_color_info: [u8; 6],
}

#[repr(C, packed)]
struct MemoryEntry {
    size: u32,
    base_addr: u64,
    length: u64,
    mtype: u32,
}

pub struct MachineContext {
    cpu_count:  usize,
    acpi_info:  AcpiInfo,
    ioapic:     X86IoApic
}

impl MachineContext {
    pub const fn new() -> Self {
        Self {
            cpu_count:  1, // There's at least one cpu (BSP), lol!
            acpi_info:  AcpiInfo::new(),
            ioapic:     X86IoApic::new()
        }
    }
}

static THIS_MACHINE: Spinlock<MachineContext> = 
    Spinlock::new(MachineContext::new());

percpu_global! {
    THIS_PERCPU_BASE: usize = 0; // To avoid rdmsr(IA32_GS_BASE) every time
    THIS_LAPIC:  X86LocalApic = X86LocalApic::new();
    THIS_IOAPIC: X86IoApic    = X86IoApic::new();
}

//----------------------------------------------------------------------------//
// Private interface between the Assembly stub (boot.S) and the Rust stub     //
//----------------------------------------------------------------------------//

//
// Kernel's entry-point for BSP (CPU0)
// boot.S must have switched to 64-bit (Long Mode) with the first
// INIT_NUM_PAGE_TABLES*2MB (up to 1GB) of the physical memory directly mapped
// to kernel's initial virtual address space
//
#[unsafe(no_mangle)]
extern "C" fn rust_x864_entry_bsp(mbi: &MultibootInfo, max_cpus: usize) {
    // Create the memory map
    let mut mem_map: [PMMapElement; 32] = [
        PMMapElement {base: 0, len: 0, avail: false}; 32];
    let e820_mmap_count = mbi.mmap_length as usize / size_of::<MemoryEntry>();
    unsafe {
        let mut rptr: *mut MemoryEntry = mbi.mmap_addr as *mut MemoryEntry;
        for i in 0..e820_mmap_count {
            mem_map[i].base = (*rptr).base_addr as usize;
            mem_map[i].len  = (*rptr).length as usize;
            mem_map[i].avail = match (*rptr).mtype {
                1 => true,
                _ => false,
            };
            rptr = rptr.add(1);
        }
    }

    // Todo - fetch kernel's boot command-line/parameters
    // Fetch VBE's information for the early-stage console/graphics driver
    vbe_init(mbi);
    vbe_set_background_rgb((240, 240, 240));
    vbe_set_foreground_rgb((0, 0, 255));
    kearly_console::init();
    let (rows, cols) = vbe_screen_size();
    log!("VESA Graphics: Mode=0x{:X}, Rows:{}, Columns:{}\n",
        vbe_mode_number(), rows, cols);
    // Todo - Pass a list kernel modules (e.g., ramdisk) Grub loaded for us
    
    // Initialize the per-cpu sections
    percpu_init_sections();
    percpu_init_cpu(0);
    *THIS_CPU_ID.borrow_mut() = 0;
    
    // SMP, LAPIC, IOAPIC, HiRes Event Timer, etc. are found in ACPI tables
    match x86_acpi_parse() {
        Some(acpi) => {
            {
                // No concurrency here, but Rust!
                let mut this_machine = THIS_MACHINE.lock();
                (*this_machine).acpi_info = acpi;
            }
            // Start the application processors
            start_smp(max_cpus);
        },
        None => {
            dbg!("No ACPI information found. Multiprocessing disabled.\n");
        }
    };
    // Start the kernel.
    kstart(0, Some(&mem_map[0..e820_mmap_count]) );
    panic!(); // kstart shouldn't really return but if does, we should panic
}

#[unsafe(no_mangle)]
extern "C" fn rust_x864_entry_ap(_arg: usize) {
    let cpuid = cpu_id();
    percpu_init_cpu(cpuid);
    THIS_CPU_ID.write(cpuid);
    let mylapic = THIS_LAPIC.borrow_mut();
    {
        let mut this_machine = THIS_MACHINE.lock();
        let acpi = &((*this_machine).acpi_info);
        for lapic in &acpi.lapic[..acpi.lapic_cnt as usize] {
            if lapic.cpu_id == cpuid as u8 {
                mylapic.init(lapic, acpi.lapic_mmio);
                break;
            }
        }
        (*this_machine).cpu_count += 1;
    }
    
    dbg!("CPU[{}] calling kstart\n", THIS_CPU_ID.borrow());
    // Initialize the LAPIC for the current CPU

    // Start the kernel for this AP
    kstart(cpuid as usize, None);
}

//
// Interrupt handler stubs
// Called by irq_/excep handler wrapper functions in boot.S ensuring proper
// system stack management and return of the control flow.
//
#[unsafe(no_mangle)]
extern "C" fn kdefault_handler() {
    kearly_console::print_str(b"kdefault_handler");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_div_by_zero() {
    panic!("kexcep_div_by_zero");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_nmi() {}

#[unsafe(no_mangle)]
extern "C" fn kexcep_overflow() {
    panic!("kexcep_overflow");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_invalid_opcode(exframe: usize) {
    let info = x86_decode_exception_frame(exframe, false);
    dump_memory(info.rsp, 8);
    panic!("#UD CPU={} RFLG={:X} CS={:X} RIP={:X} SS={:X} RSP={:X}",
        info.cpu, info.rflg, info.cs, info.rip, info.ss, info.rsp
    );
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_invalid_tss() {
    panic!("kexcep_invalid_tss");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_seg_not_present() {
    panic!("kexcep_seg_not_present");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_stack_fault() {
    panic!("kexcep_stack_fault");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_gp_fault(exframe: usize) {
    let info = x86_decode_exception_frame(exframe, true);
    dump_memory(info.rsp, 8);
    panic!("#GP CPU={} ERR={:X} RFLG={:X} CS={:X} RIP={:X} SS={:X} RSP={:X}",
            info.cpu, info.err, info.rflg, info.cs, info.rip, info.ss, info.rsp
    );
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_page_fault(exframe: usize) {
    let info = x86_decode_exception_frame(exframe, true);
    dump_memory(info.rsp, 8);
    panic!("#PF CPU={} CR2={:X} ERR={:X} RFLG={:X} CS={:X} RIP={:X} SS={:X} RSP={:X}",
        info.cpu, info.cr2, info.err, info.rflg, info.cs, info.rip,
        info.ss, info.rsp
    );

}

#[unsafe(no_mangle)]
extern "C" fn kexcep_x87fpu() {
    panic!("kexcep_x87fpu");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_alignment_chk() {
    panic!("kexcep_alignment_chk");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_machine_chk() {
    panic!("kexcep_machine_chk");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_simd_fp() {
    panic!("kexcep_simd_fp");
}

#[unsafe(no_mangle)]
extern "C" fn kirq_handler(irq: u8) {
    // For simplicity: EOI immediately not to lose IRQs in case the top-half
    // handler ends up doing a context switch or takes too long.

    THIS_LAPIC.borrow_mut().send_eoi();

    unsafe {X86_ISR_HANDLER[irq as usize](irq as u16);}
}

#[unsafe(no_mangle)]
extern "C" fn kstack_error(rsp: usize) {
    dump_memory(rsp, 20);
    panic!("Kernel Stack Corruption.");
}

#[unsafe(no_mangle)]
extern "C" fn ksyscall_handler() {}

#[derive(Default)]
struct X86ExceptionInfo {
    cpu:    usize,
    err:    usize,
    cs:     usize,
    rip:    usize,
    rflg:   usize,
    rsp:    usize,
    ss:     usize,
    cr2:    usize,
    cr4:    usize
}

fn x86_decode_exception_frame(exframe: usize, error_code: bool) -> 
X86ExceptionInfo {
    unsafe {
        let (cpu, cr2, cr4): (usize, usize, usize);
        cpu = cpu_id();
        asm!("mov rax, cr2", out("rax")cr2);
        asm!("mov rax, cr4", out("rax")cr4);
        if error_code {
            X86ExceptionInfo {
                cpu : cpu,
                err : *((exframe + 8 * 0) as *const usize),
                rip : *((exframe + 8 * 1) as *const usize),
                cs  : *((exframe + 8 * 2) as *const usize),
                rflg: *((exframe + 8 * 3) as *const usize),
                rsp : *((exframe + 8 * 4) as *const usize),
                ss  : *((exframe + 8 * 5) as *const usize),
                cr2 : cr2,
                cr4 : cr4
            }
        } else {
            X86ExceptionInfo {
                cpu : cpu,
                err : 0,
                rip : *((exframe + 8 * 0) as *const usize),
                cs  : *((exframe + 8 * 1) as *const usize),
                rflg: *((exframe + 8 * 2) as *const usize),
                rsp : *((exframe + 8 * 3) as *const usize),
                ss  : *((exframe + 8 * 4) as *const usize),
                cr2 : cr2,
                cr4 : cr4
            }
        }
    }
}

//----------------------------------------------------------------------------//
// Internal interface                                                         //
//----------------------------------------------------------------------------//

//
// PerCpu Storage Support - util::PerCpuGlobal<T> requires the following
// architecture-dependent functions to be defined here:
// percpu_borrow and percpu_borrow_mut
// The rest of the kernel code should not use these interfaces as they are not
// thread- or type-safe
//
const IA32_GS_BASE: u32 = 0xC0000101;
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
            raw_memcpy(pcpu_s + s * sect_size, pcpu_s, sect_size);
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
        x86_msr_write(IA32_GS_BASE, base_addr as u64);
        asm!(
            "mov gs:{base}, rax",
            base = sym THIS_PERCPU_BASE,
            in("rax")base_addr
        );
    }
}
pub fn percpu_borrow<T>(var: &T) -> &T {
    unsafe {
        let mut addr : usize;
        asm!("mov rax, gs:{base}", base = sym THIS_PERCPU_BASE, out("rax")addr);
        addr = addr + var as *const T as usize;
        &(*(addr as *mut T))
    }
}
pub fn percpu_borrow_mut<T>(var: &T) -> &mut T {
    unsafe {
        let mut addr : usize;
        asm!("mov rax, gs:{base}", base = sym THIS_PERCPU_BASE, out("rax")addr);
        addr = addr + var as *const T as usize;
        &mut *(addr as *mut T)
    }
}

//
// Base x86 Drivers
// + Programmable Interrupt Controller (PIC)
// + Programmable Interval Timer (PIT)
//
mod x86_pic {
    #![allow(dead_code)]
    use crate::arch::{x86_ioport_read, x86_ioport_write};

    // IO Ports
    const PIC1_PORT_CMD: u16 = 0x20;
    const PIC1_PORT_DAT: u16 = 0x21;
    const PIC2_PORT_CMD: u16 = 0xA0;
    const PIC2_PORT_DAT: u16 = 0xA1;
    // Commands
    const PIC_CMD_EOI:  u8 = 0x20;
    const PIC_CMD_INIT: u8 = 0x11;

    pub fn init(idt_vector_offset: u8) {
        let (mask1, mask2): (u8, u8);

        // Save IRQ masks
        mask1 = x86_ioport_read(PIC1_PORT_DAT);
        mask2 = x86_ioport_read(PIC2_PORT_DAT);

        // Send the initialization command and data sequence 
        x86_ioport_write(PIC1_PORT_CMD, PIC_CMD_INIT);
        x86_ioport_write(PIC2_PORT_CMD, PIC_CMD_INIT);
        // Set the vector offests
        x86_ioport_write(PIC1_PORT_DAT, idt_vector_offset);
        x86_ioport_write(PIC2_PORT_DAT, idt_vector_offset + 8);
        // IRQ2 on the master PIC is connected to the slave (PIC2)
        x86_ioport_write(PIC1_PORT_DAT, 4);
        // Cascade ID of the slave
        x86_ioport_write(PIC2_PORT_DAT, 2);
        // Set the mode of both PICs to 8086
        x86_ioport_write(PIC1_PORT_DAT, 1);
        x86_ioport_write(PIC2_PORT_DAT, 1);

        // Restore the IRQ masks
        x86_ioport_write(PIC1_PORT_DAT, mask1);
        x86_ioport_write(PIC2_PORT_DAT, mask2);
    }

    pub fn send_eoi(irq: u8) {
        if irq >= 8 {
            x86_ioport_write(PIC2_PORT_CMD, PIC_CMD_EOI);
        }
        x86_ioport_write(PIC1_PORT_CMD, PIC_CMD_EOI);
    }

    pub fn mask_irq(irq: u8) {
        if irq < 8 {
            x86_ioport_write(PIC1_PORT_DAT, 
                x86_ioport_read(PIC1_PORT_DAT) | (1 << irq));
        } else {
            x86_ioport_write(PIC2_PORT_DAT, 
                x86_ioport_read(PIC2_PORT_DAT) | (1 << (irq - 8)));
        }
    }

    pub fn mask_all() {
        x86_ioport_write(PIC1_PORT_DAT, 0xFF);
        x86_ioport_write(PIC2_PORT_DAT, 0xFF);
    }

    pub fn unmask_irq(irq: u8) {
        if irq < 8 {
            x86_ioport_write(PIC1_PORT_DAT, 
                x86_ioport_read(PIC1_PORT_DAT) & !(1 << irq));
        } else {
            x86_ioport_write(PIC2_PORT_DAT, 
                x86_ioport_read(PIC2_PORT_DAT) & !(1 << (irq - 8)));
        }
    }
}

mod x86_pit {
    #![allow(dead_code)]
    use crate::arch::x86_ioport_write;

    // I/O Ports
    const PIT_PORT_CH0: u16 = 0x40;
    const PIT_PORT_CH1: u16 = 0x41;
    const PIT_PORT_CH2: u16 = 0x42;
    const PIT_PORT_CMD: u16 = 0x43;
    // Command Fields
    const PIT_ACCESS_LOW_BYTE: u8 = 0x1;
    const PIT_ACCESS_HI_BYTE : u8 = 0x2;
    const PIT_ACCESS_LOW_HI  : u8 = PIT_ACCESS_LOW_BYTE | PIT_ACCESS_HI_BYTE;
    const PIT_OPMODE_RATEGEN : u8 = 0x2;
    // Other constants
    const PIT_FREQ_HZ: u32 = 1193182;

    fn make_cmd(channel: u8, access: u8, opmode: u8) -> u8 {
        ((channel & 0x3) << 6 ) | 
        ((access  & 0x3) << 4 ) |
        ((opmode  & 0x7) << 1 )
    }

    pub fn init(hz: u16) {
        let reload : u16 = (PIT_FREQ_HZ / hz as u32) as u16;
        let cmd = make_cmd(0, PIT_ACCESS_LOW_HI, PIT_OPMODE_RATEGEN);
        x86_ioport_write(PIT_PORT_CMD, cmd);
        x86_ioport_write(PIT_PORT_CH0, (reload & 0xFF) as u8);
        x86_ioport_write(PIT_PORT_CH0, (reload >> 8)   as u8);
    }
}

struct X86IoApic{
    acpi_id:        u8,
    version:        u8,
    max_irqs:       u8,
    initialized:    bool,
    mmio_base:      u32,
    gsi_base:       u32
}
impl X86IoApic {
    const REG_ID:    u8 = 0x0;
    const REG_VER:   u8 = 0x1;

    // IRQ Delivery Priority Values
    pub const PRIORITY_FIXED:   u32 = 0x0;
    pub const PRIORITY_LOWEST:  u32 = 0x100;
    pub const PRIORITY_SMI:     u32 = 0x200;
    pub const PRIORITY_NMI:     u32 = 0x400;
    pub const PRIORITY_INIT:    u32 = 0x500;
    pub const PRIORITY_EXTINT:  u32 = 0x700;
    // IRQ Pin Polarity 
    pub const POLARITY_HIGH:    u32 = 0x0;
    pub const POLARITY_LOW:     u32 = 0x2000;
    // IRQ Pin Trigger Mode
    pub const TRIGGER_EDGE:     u32 = 0x0;
    pub const TRIGGER_LEVEL:    u32 = 0x8000;

    pub const fn new() -> Self {
        Self {
            acpi_id:    0,
            gsi_base:   0,
            initialized:false,
            max_irqs:   0,
            mmio_base:  0,
            version:    0
        }
    }

    fn read_reg(&self, reg_index: u8) -> u32 {
        if self.initialized == false { return 0; }
        let io_reg_sel : *mut u32 = self.mmio_base as *mut u32;
        let io_reg_dat : *mut u32 = (self.mmio_base + 0x10) as *mut u32;
        unsafe {
            io_reg_sel.write_volatile(reg_index as u32);
            io_reg_dat.read_volatile()
        }
    }

    fn write_reg(&self, reg_index: u8, value: u32) {
        if !self.initialized { return; }
        let io_reg_sel : *mut u32 = self.mmio_base as *mut u32;
        let io_reg_dat : *mut u32 = (self.mmio_base + 0x10) as *mut u32;
        unsafe {
            io_reg_sel.write_volatile(reg_index as u32);
            io_reg_dat.write_volatile(value);
        }
    }

    pub fn init(&mut self, ioapic: &AcpiIOApic) {
        // TODO identity-map mmio_base with caching disabled
        // The MMIO is usually toward the end of the 4GB boundary. Don't accept
        // an MMIO mapping under 1MB.
        if ioapic.ioapic_mmio <= 0x100000 { return; }
        self.initialized = true;
        self.mmio_base = ioapic.ioapic_mmio;
        self.gsi_base  = ioapic.gsi_base;
        self.acpi_id = (self.read_reg(Self::REG_ID) >> 24) as u8;
        let ver = self.read_reg(Self::REG_VER);
        self.version = (ver & 0xFF) as u8;
        self.max_irqs= ((ver >> 16) & 0xFF) as u8 + 1;

        dbg!("IOAPIC init => ID:{} MMIO: {:X} Ver:{} Max IRQs:{}\n", 
            self.acpi_id, self.mmio_base, self.version, self.max_irqs
        );
    }

    pub fn register_isr(&mut self, gsi: u32, isr_vector: u8,
                        priority: u32, pin_polarity: u32, pin_trigger: u32,
                        masked: bool, dest_cpu_acpi_id_mask: u8)
    {
        let entry_index = gsi - self.gsi_base;
        if gsi < self.max_irqs as u32 {
            let (mut high, low) : (u32, u32);
            low = (isr_vector as u32) | priority | pin_polarity | pin_trigger;
            high = (dest_cpu_acpi_id_mask as u32) << 24;
            if masked {high |= 1;}  
            self.write_reg((entry_index * 2 + 0x10) as u8, low);
            self.write_reg((entry_index * 2 + 0x11) as u8, high);
        }

    }

    pub fn set_irq_mask(&mut self, gsi: u32, masked: bool) {
        let entry_index = gsi - self.gsi_base;
        if gsi < self.max_irqs as u32 {
            let mut high = self.read_reg((entry_index * 2 + 0x11) as u8);
            high = match masked {
                true  => high | 0x1,
                false => high & 0xFFFFFFFE
            };
            self.write_reg((entry_index * 2 + 0x11) as u8, high);
        }
    }
}

struct X86LocalApic {
    lapic_id:   u8,
    cpu_acpi_id:u8,
    mmio_base:  u32,
    initialized:bool,
}
impl X86LocalApic {

    const REG_LAPIC_ID:         u16 = 0x20;
    const REG_LAPIC_VERSION:    u16 = 0x30;
    const REG_TASK_PRIORITY:    u16 = 0x80;
    const REG_ARB_PRIORITY:     u16 = 0x90;
    const REG_PROC_PRIORITY:    u16 = 0xA0;
    const REG_EOI:              u16 = 0xB0;
    const REG_SIV:              u16 = 0xF0; // Spurious Int Vector Reg
    const REG_ERROR_STATUS:     u16 = 0x280;
    const REG_INT_CMD1:         u16 = 0x300;
    const REG_INT_CMD2:         u16 = 0x310;
    const REG_LVT_TIMER:        u16 = 0x320;
    const REG_LVT_LINT0:        u16 = 0x350;
    const REG_LVT_LINT1:        u16 = 0x360;
    const REG_LVT_ERROR:        u16 = 0x370;
    const REG_TIMER_INIT_CNT:   u16 = 0x380;
    const REG_TIMER_CUR_CNT:    u16 = 0x390;
    const REG_TIMER_DIV:        u16 = 0x3E0;

    pub const fn new() -> Self {
        Self {
            cpu_acpi_id: 0,
            initialized: false,
            lapic_id:    0,
            mmio_base:   0
        }
    }

    fn read_reg(&mut self, reg: u16) -> u32 {
        let regref : *mut u32 = (self.mmio_base + reg as u32) as *mut u32;
        unsafe {
            regref.read_volatile()
        }
    }

    fn write_reg(&mut self, reg: u16, value: u32) {
        let regref : *mut u32 = (self.mmio_base + reg as u32) as *mut u32;
        unsafe {
            regref.write_volatile(value);
        }
    }

    fn send_ipi(&mut self, dest_lapic_id: u8, cmd1: u32) {
        // Select the target LAPIC
        let reg = self.read_reg(Self::REG_INT_CMD2) & 0x00FFFFFF;
        self.write_reg(Self::REG_INT_CMD2, reg | (dest_lapic_id as u32)<<24);
        // Send the command
        let reg = self.read_reg(Self::REG_INT_CMD1) & 0xFFF00000;
        self.write_reg(Self::REG_INT_CMD1, reg | cmd1);

    }

    fn wait_ipi_send(&mut self){
        // Poll CMD1.bits[12], which clears when the IPI is accepted by the dest
        // Todo - timeout here instead of an endless loop
        while self.read_reg(Self::REG_INT_CMD1) & 0x1000 > 0 {
            unsafe {asm!("pause");}
        }
    }

    pub fn init(&mut self, lapic: &AcpiLocalApic, lapic_mmio: u32) {
        // TODO identity-map mmio_base with caching disabled
        // The MMIO is usually toward the end of the 4GB boundary. Don't accept
        // an MMIO mapping under 1MB.
        if lapic_mmio <= 0x100000 { return; }
        self.lapic_id       = lapic.lapic_id;
        self.cpu_acpi_id    = lapic.cpu_id;
        self.mmio_base      = lapic_mmio;
        self.initialized    = true;

        // Setting bit 8 of the spurious interrupt vector enables the lapic
        let siv = self.read_reg(Self::REG_SIV);
        self.write_reg(Self::REG_SIV, siv | 0x100);
    }

    pub fn send_eoi(&mut self) {
        self.write_reg(Self::REG_EOI, 0);
    }

    pub fn send_init_ipi(&mut self, dest_lapic_id: u8) {
        // clear errors
        self.write_reg(Self::REG_ERROR_STATUS, 0);
        // Assert the IPI signal (Delivery: INIT, Assert)
        self.send_ipi(dest_lapic_id, 0xC500);
        self.wait_ipi_send();
        // De-assert the IPI signal
        self.send_ipi(dest_lapic_id, 0x8500);
        self.wait_ipi_send();
    }

    pub fn send_startup_ipi(&mut self, dest_lapic_id: u8, entry_point_pg: u8) {
        // clear errors
        self.write_reg(Self::REG_ERROR_STATUS, 0);
        // Send SIPI
        self.send_ipi(dest_lapic_id, 0x600 | entry_point_pg as u32);
		cpu_busywait(2_000_000); // TODO: precise 200 uS
		self.wait_ipi_send();
    }
}

//----------------------------------------------------------------------------//
// External interface exposed to kernel's general code                        //
//----------------------------------------------------------------------------//
percpu_global!{
    pub THIS_CPU_ID: usize = 0; // To avoid issuing cpuid every time
}


//
// Assembly Wrapper functions
//
pub fn cpu_enable_ints() {
    unsafe {
        asm!("sti");
    }
}
pub fn cpu_disable_ints() {
    unsafe {
        asm!("cli");
    }
}
pub fn cpu_halt() {
    unsafe {
        asm!("hlt");
    }
}

pub fn cpu_count() -> usize {
    (*(THIS_MACHINE.lock())).cpu_count
}

// Returns LAPIC CPU ID of the current CPU calling the routine using the
// CPUID instruction with Extended Topology Leaf (0BH)
pub fn cpu_id() -> usize {
    unsafe {
        let apic_id: u32;
        asm!(
            "mov    eax, 0xb",
            "mov    ecx, 0x0",
            "cpuid",
            // EDX should hold the APIC ID
            out("edx")apic_id
        );
        apic_id as usize
    }
}


pub fn x86_cpuid_inst(in_eax: u32, in_ecx: u32) -> (u32, u32, u32, u32) {
    let (mut eax, mut ebx, mut ecx, mut edx) : (u32, u32, u32, u32);
    eax = in_eax;
    ecx = in_ecx;
    unsafe {
        asm!(
            "push rbx", // rbx is used internally by LLVM
            "cpuid",
            "mov {tmp:e}, ebx",
            "pop rbx",
            inout("eax")eax,
            inout("ecx")ecx,
            tmp = out(reg)ebx,
            out("edx")edx
        );
    }
    (eax, ebx, ecx, edx)
}

pub fn x86_cpuid_max_extended_leaf() -> u32 {
    // CPUID.80000000H -- Maximum Input Value for Extended Function CPUID
    // Information
    let eax;
    (eax, _, _, _) = x86_cpuid_inst(0x80000000, 0);
    eax
}

pub fn cpu_read_timestamp() -> u64 {
    let (upper, lower): (u64, u64);
    unsafe {
        asm!("rdtsc", out("rdx")upper, out("rax")lower);
    }
    (upper << 32) | lower
}
pub fn cpu_busywait(delay_tsc: u64) {
    let target_tsc = cpu_read_timestamp() + delay_tsc;
    while cpu_read_timestamp() < target_tsc {
        core::hint::spin_loop();
    }
}
pub fn x86_ioport_read(port: u16) -> u8 {
    let data: u8;
    unsafe {
        asm!("in al, dx", out("al") data, in("dx") port);
    }
    data
}
pub fn x86_ioport_write(port: u16, data: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") data);
    }
}

pub fn x86_msr_write(msr: u32, val: u64) {
    unsafe {
        asm!(
            "wrmsr",
            in("rcx") msr,
            in("rdx") val >> 32,
            in("rax") val & 0xFFFFFFFF,
        );
    }
}

pub fn x86_msr_read(msr: u32) -> u64 {
    let (high, low) : (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("rcx") msr,
            out("rdx") high,
            out("rax") low,
        );
    }
    (high as u64) << 32 | low as u64
}

//
// Initial/Boot-time Console
// VGA/80x24TXT mode
// Provides a print_str to klog! or similar debug/log printing routines.
// print_str is implemented in a synchronizing manner
//
pub mod kearly_console {
    use crate::arch::asc::vga::*;
    use crate::util::Spinlock;

    static CURSOR : Spinlock<(u32, u32)> = Spinlock::new((0,0));
    // CURSOR.0 -> row, .1 -> column

    pub fn init() {
        vbe_clean_screen();
    }

    pub fn print_str(msg: &[u8]) {
        let (sh, sw) = vbe_screen_size();
        let (fh, fw) = vbe_font_size();
        let (rows, cols) = (sh / fh, sw / fw);
        let mut cursor = CURSOR.lock();
        for &c in msg {
            if c == b'\n' {
                (*cursor).0 = ((*cursor).0 + 1) % rows;
                (*cursor).1 = 0;
            } else {
                vbe_putc(c, (*cursor).0, (*cursor).1);
                (*cursor).1 = (*cursor).1 + 1;
                if (*cursor).1 == cols {
                    (*cursor).1 = 0;
                    (*cursor).0 = ((*cursor).0 + 1) % rows;
                }
            }
        }
    }
}

//
// System Timer(s)
//
pub fn systimer_set_periodic(freq_hz: u16, isr_callback: IsrHandlerFn) {
    isr_register(0, isr_callback);
    x86_pit::init(freq_hz);
}

//
// IRQ Interface
//
type IsrHandlerFn = fn(u16);

fn isr_default_imp(_: u16) { }

static mut X86_ISR_HANDLER: [IsrHandlerFn; 16] = [isr_default_imp; 16];

pub fn irq_controller_init() {
    x86_pic::init(32);
}

pub fn isr_register(irq: u16, handler_fn: IsrHandlerFn) {
    if irq < 16 {
        unsafe {
            X86_ISR_HANDLER[irq as usize] = handler_fn;
        }
    }
}

//
// Memory Management Unit (MMU) Primitives
//
pub fn mmu_page_size() -> usize {
    0x1000
}

pub fn mmu_addr_to_page_index(addr: usize) -> usize {
    addr >> 12
}

pub enum MmuCachingPolicy {
    NonCaching,    // Slow Memory RW - Totally safe for MMIO/DMA
    WriteThrough,  // Fast Memory R, Slow Memory/MMIO W - Not safe for MMIO Read
    WriteBack      // Fast Memory-only R/W - Not for MMIO
}
// Maps a page (virtual address) to a frame (physical address) in the current
// address-space (CR3)
pub fn mmu_map_page(_virt_addr: usize, _phys_addr: usize, _privileged: bool,
            _writeable: bool, _executable: bool, _caching: MmuCachingPolicy) {
    // Todo - Should look at CR3, map the structs into a temporary scratchpad
    //        space and then add the new map.
    // Forgot how MTRR registers affect specific memory region caching...
    // Look it up.
    
}

pub fn mmu_unmap_page(_virt_addr: usize) {

}

//
// Symmetric Multiprocessing Support
//
fn start_smp(max_cpus: usize) {
    let lapic_count;
    let lapic0 =  THIS_LAPIC.borrow_mut();
    {
        let this_machine = THIS_MACHINE.lock();
        lapic_count = (*this_machine).acpi_info.lapic_cnt;

        let acpi = &(*this_machine).acpi_info;
        // Print what we found on ACPI tables if compiled with debug_arch
        // CPUs/LAPICS
        dbg!("LAPIC MMIO @ {:X}\n", acpi.lapic_mmio);
        for _lapic in &acpi.lapic[..acpi.lapic_cnt as usize] {
            dbg!("CPU[{}]: LAPIC ID: {}, Enabled: {}\n",
                _lapic.cpu_id,
                _lapic.lapic_id,
                _lapic.enabled
            );
        }
        // IOAPIC
        dbg!("IOAPIC[{}]: MMIO Base: {:X}, GSI Base: {:X}\n",
            acpi.ioapic.ioapic_id,
            acpi.ioapic.ioapic_mmio,
            acpi.ioapic.gsi_base
        );
        // IRQ->GSI mappings
        for _irq in &acpi.irq_map[..acpi.irq_map_cnt as usize] {
            dbg!("<IRQ#{}.{} -> GSI#{} ON {}{}> ",
                _irq.src_bus, _irq.src_irq, _irq.dst_gsi,
                if _irq.active_low {"Low"} else {"High"},
                if _irq.lvl_trig {"Level"} else {"Edge"}
            );
        }
        // NMI->LINT mappings
        for _i in 0..acpi.nmi_map_cnt as usize {
            dbg!("<NMI.CPUS[{:X}] -> LINT#{} ON {}{}> ",
                acpi.nmi_map[_i].cpu_id_mask,
                acpi.nmi_map[_i].lint_vector,
                if acpi.nmi_map[_i].active_low {"Low"} else {"High"},
                if acpi.nmi_map[_i].lvl_trig {"Level"} else {"Edge"}
            );
        }
        dbg!("\n");
        //// Initialize the LocaAPIC controller for BSP (CPU 0)
        for lapic in &acpi.lapic[..acpi.lapic_cnt as usize] {
            if lapic.cpu_id == 0 {
                lapic0.init(lapic, acpi.lapic_mmio);
                break;
            }
        }
    }

    // Start the Application Processors 
    //// 1) Copy the trampoline code into 0x8000
    unsafe extern "C" {
        static _AP_STARTUP16_ENTRY: usize;
        static _AP_STARTUP16_END:   usize;
        static _KINIT_STACK_START:  usize;
    }
    unsafe {
        let src_start: usize = &_AP_STARTUP16_ENTRY as *const usize as usize;
        let src_end  : usize = &_AP_STARTUP16_END as *const usize as usize;
        raw_memcpy(0x8000, src_start, src_end - src_start);
    }
    //// 2) Send INIT-SIPI_SIPI for each AP and check the magic# on their stack
    ////    That would indicate the completion of their trampoline code.
    dbg!("MAX CPUs: {}\n", max_cpus);
    for i in 0..lapic_count as usize {
        let lapic_id;
        {
            let this_machine = THIS_MACHINE.lock();
            lapic_id = (*this_machine).acpi_info.lapic[i].cpu_id;
        }
        if lapic_id > 0 && lapic_id < max_cpus as u8 {
            let current_cpu_cnt = cpu_count();
            dbg!("Senging INIT-SIPI to CPU[{}]\n", lapic_id);
            lapic0.send_init_ipi(lapic_id);
            cpu_busywait(10_000_000); // Wait for the cpu to initialize (~10ms)
            lapic0.send_startup_ipi(lapic_id, 0x8); 
            cpu_busywait(10_000_000); // Wait for the AP to initialize
            if cpu_count() == current_cpu_cnt {
                // Send another SIPI
                dbg!("Sending another SIPI to CPU[{}]\n", lapic_id);
                lapic0.send_startup_ipi(lapic_id, 0x8);
            }
           
            loop {
                cpu_busywait(1_000_000);
                if cpu_count() > current_cpu_cnt {break;}
            }
        }
    }
    // Initialize IOAPIC: 
    //// Disable PIC
    x86_pic::mask_all();
    //// Set up IOAPIC[0] and IRQ->GSI redirection table
    //// Fill in all available GSIs according to ACPI info, and then:
    //// - map them to their corresponding IDT vector
    //// - mask them. The generic code should enable each IRQ when the 
    ////   corresponding driver is initialized!
    //// - route them to CPU0 by default.
    let ioapic0 = THIS_IOAPIC.borrow_mut();
    let isr_vector_offset = 32; // See how IDT is set up in boot.S
    {
        let this_machine = THIS_MACHINE.lock();
        let acpi = &(*this_machine).acpi_info;
        ioapic0.init(&acpi.ioapic);
        for irq in &acpi.irq_map[0..acpi.irq_map_cnt as usize] {
            ioapic0.register_isr(
                irq.dst_gsi, irq.src_irq + isr_vector_offset,
                X86IoApic::PRIORITY_FIXED,
                if irq.active_low {X86IoApic::POLARITY_LOW} 
                             else {X86IoApic::POLARITY_HIGH},
                if irq.lvl_trig {X86IoApic::TRIGGER_LEVEL}
                             else {X86IoApic::TRIGGER_EDGE},
                false, 0
            );
        }
    }
    //// TODO Route NMIs

}

//
// System Configuration (ACPI)
//
pub const MAX_CPU_COUNT: usize = 8;
pub const MAX_IRQ_COUNT: usize = 16;

struct AcpiInfo {
    madt_base:  u32,
    fadt_base:  u32,
    hpet_base:  u32,
    lapic_cnt:  u32,
    lapic_mmio: u32, // Usually 0xFEE00000
    lapic:      [AcpiLocalApic; MAX_CPU_COUNT],
    ioapic:     AcpiIOApic, // No support for multiple sockets/IOAPICs
    irq_map_cnt:u32,
    irq_map:    [AcpiIRQMapping; MAX_IRQ_COUNT],
    nmi_map_cnt:u32,
    nmi_map:    [AcpiNmiMapping; MAX_CPU_COUNT], // At most: 1 NMI-mapping/CPU
}
impl AcpiInfo {
    pub const fn new() -> Self {
        Self {
            fadt_base:  0,
            hpet_base:  0,
            ioapic:     AcpiIOApic::new(),
            irq_map:    [AcpiIRQMapping::new(); MAX_IRQ_COUNT],
            irq_map_cnt:0,
            lapic:      [AcpiLocalApic::new(); MAX_CPU_COUNT],
            lapic_cnt:  0,
            lapic_mmio: 0,
            madt_base:  0,
            nmi_map:    [AcpiNmiMapping::new(); MAX_CPU_COUNT],
            nmi_map_cnt: 0
        }
    }
}

#[derive(Clone, Copy)]
struct AcpiLocalApic {
    cpu_id:     u8,
    lapic_id:   u8,
    enabled:    bool
}
impl AcpiLocalApic {
    pub const fn new() -> Self {
        Self {
            cpu_id:     0,
            enabled:    false,
            lapic_id:   0
        }
    }
}

struct AcpiIOApic {
    ioapic_id:  u8,
    ioapic_mmio:u32,
    gsi_base:   u32 // must be 0 in a single IOAPIC config
}
impl AcpiIOApic {
    pub const fn new() -> Self {
        Self {
            gsi_base:   0,
            ioapic_id:  0,
            ioapic_mmio:0
        }
    }
}

#[derive(Clone, Copy)]
struct AcpiIRQMapping {
    src_bus:    u8,
    src_irq:    u8,
    dst_gsi:    u32,
    active_low: bool, // Active Low or Active High signal
    lvl_trig:   bool, // Triggered on the Level or on the Edge of the signal
}
impl AcpiIRQMapping {
    pub const fn new() -> Self {
        Self {
            active_low: false,
            dst_gsi:    0,
            lvl_trig:   false,
            src_bus:    0,
            src_irq:    0
        }
    }
}

#[derive(Clone, Copy)]
struct AcpiNmiMapping {
    cpu_id_mask:u8,
    lint_vector:u8, // Entry# of the vector table of CPUs' LAPIC
    active_low: bool,
    lvl_trig:   bool
}

impl AcpiNmiMapping {
    pub const fn new() -> Self {
        Self {
            active_low: false,
            cpu_id_mask:0,
            lint_vector:0,
            lvl_trig:   false
        }
    }
}

fn x86_acpi_parse() -> Option<AcpiInfo>{
    // 1) Find "RSD PTR " in low memory - on a 
    let mut ptr: *mut u64 = 400 as *mut u64;
    let mut valid_rsdp = false;

    for _i in 0..0x20000 {
        unsafe {
            if *ptr == 0x2052545020445352 { 
                valid_rsdp = true;
                break;
            }
            ptr = ptr.wrapping_add(2); // 16-byte boundary
        }
    }
    if valid_rsdp == false {
        return None;
    }
    let mut ret = AcpiInfo::new();
    // 2) Find RSDT (RSD[16] as u32)
    // FORMAT OF THE ROOT RSDT
    // OFF TYPE&NAME
    // 0   char Signature[4]; <-- "RSDT"
    // 4   uint32_t Length;   <-- Total size of the table including this header
    // 8   uint8_t Revision;
    // 9   uint8_t Checksum;
    // 10  char OEMID[6];
    // 16  char OEMTableID[8];
    // 24  uint32_t OEMRevision;
    // 28  uint32_t CreatorID;
    // 32  uint32_t CreatorRevision;
    // 36  u32 address of the next SDT <-- Starting at offset:
    // 40  u32 address of the another SDT
    // ....
    // u32 address of the last SDT
    unsafe {

        let mut rsdt: *mut u32 = *(ptr.wrapping_add(2)) as *mut u32;
        let num_tables : u32 = (*rsdt.wrapping_add(1) - 36) / 4;
        // let acpi_ver: u8 = *(rsdt.wrapping_add(2) as *mut u8);
        // archlog!("ACPI v{} (RSDT) @{:p}. SIG: {:X}, LEN:{} #TBLS: {}\n", 
        //             acpi_ver, rsdt, *rsdt, *(rsdt.wrapping_add(1)), num_tables);
        rsdt = rsdt.wrapping_add(9);
        for _ in 0..num_tables {
            let sdt: *mut u32 = *rsdt as *mut u32;
            // System Description Table header signatures to look for:
            // "APIC" = 0x43495041 => MADT (Multi-APIC Description Table)
            // "FACP" = 0x50434146 => FADT (Fixed ACPI Description Table)
            // "HPET" = 0x54455048 => HPET (High Resolution Event Timer)
            match *sdt {
                0x43495041 => x86_acpi_parse_madt(&mut ret, sdt as u32), 
                0x50434146 => x86_acpi_parse_facp(&mut ret, sdt as u32),
                0x54455048 => x86_acpi_parse_hpet(&mut ret, sdt as u32),
                _ => ()
            };
            rsdt = rsdt.wrapping_add(1);

        }
    }
    Some(ret)
}

fn x86_acpi_parse_madt(acpi: &mut AcpiInfo, madt_addr: u32) {
    unsafe {
        let madt: *mut u32 = madt_addr as *mut u32;
        let madt_len = *(madt.wrapping_add(1));

        // Save the base address of MADT for future use?
        acpi.madt_base = madt_addr;

        // LocalAPIC base address (ignoring lapic flags)
        acpi.lapic_mmio = *(madt.wrapping_add(9));

        // The rest of the entries: CPU, IOAPIC, GSI Mapping, NMI Mapping
        let mut lapic_cnt: usize = 0;
        let mut irq_cnt: usize = 0;
        let mut nmi_cnt: usize = 0;
        let mut entry_addr = madt.wrapping_add(11) as u32;
        while entry_addr < madt_addr + madt_len {
            let entry: *mut u8 = entry_addr as *mut u8;
            let entry_type: u8 = *entry;
            let entry_len:  u8 = *(entry.wrapping_add(1));
            match entry_type {
                0 => {
                    acpi.lapic[lapic_cnt].cpu_id    = *(entry.wrapping_add(2));
                    acpi.lapic[lapic_cnt].lapic_id  = *(entry.wrapping_add(3));
                    acpi.lapic[lapic_cnt].enabled   = 
                                                 *(entry.wrapping_add(4)) == 1;
                    lapic_cnt += 1;
                },
                1 => {
                    // Only record the first IOAPIC
                    if acpi.ioapic.ioapic_mmio == 0 {
                        acpi.ioapic.ioapic_id   = *(entry.wrapping_add(2));
                        acpi.ioapic.ioapic_mmio = 
                                        *(entry.wrapping_add(4) as *mut u32);
                        acpi.ioapic.gsi_base = 
                                        *(entry.wrapping_add(8) as *mut u32);
                    }
                },
                2 => {
                    acpi.irq_map[irq_cnt].src_bus = 
                                        *(entry.wrapping_add(2));
                    acpi.irq_map[irq_cnt].src_irq =
                                        *(entry.wrapping_add(3));
                    acpi.irq_map[irq_cnt].dst_gsi =
                                        *(entry.wrapping_add(4) as *mut u32);
                    acpi.irq_map[irq_cnt].active_low = 
                        (*(entry.wrapping_add(8) as *mut u16) & 0x2) == 0x2;
                    acpi.irq_map[irq_cnt].lvl_trig =
                        (*(entry.wrapping_add(8) as *mut u16) & 0x8) == 0x8;
                    irq_cnt += 1;
                },
                4 => {
                    acpi.nmi_map[nmi_cnt].cpu_id_mask =*(entry.wrapping_add(2));
                    acpi.nmi_map[nmi_cnt].lint_vector =*(entry.wrapping_add(5));
                    acpi.nmi_map[nmi_cnt].active_low = 
                        (*(entry.wrapping_add(3) as *mut u16) & 0x2) == 0x2;
                    acpi.nmi_map[nmi_cnt].lvl_trig =
                        (*(entry.wrapping_add(3) as *mut u16) & 0x8) == 0x8;
                    nmi_cnt += 1;
                }
                _ => {
                    dbg!("MADT Entry T[{}] Ignored\n ",entry_type);
                }
            };
            entry_addr += entry_len as u32;
        }
        acpi.lapic_cnt      = lapic_cnt as u32;
        acpi.irq_map_cnt    = irq_cnt as u32;
        acpi.nmi_map_cnt    = nmi_cnt as u32;
    }
}

fn x86_acpi_parse_facp(acpi: &mut AcpiInfo, addr: u32) {
    acpi.fadt_base = addr;
    // Todo - Extract useful stuff such as reset vector, power ports, etc.
}

fn x86_acpi_parse_hpet(acpi: &mut AcpiInfo, addr: u32) {
    acpi.hpet_base = addr;
    // Todo - Extract HPET's info to use as the kernel's ticker instead of PIT
}

//
// Task Management
// + Context creation
// + Context switch
//
#[derive(Copy, Clone)]
#[repr(C)]
pub struct TaskContext {
    ep:     fn(),   // Initial RIP value, i.e., Entry-point
    rsp:    usize,  // Last RSP (Stack Pointer) value
    pas:    usize,  // Process Address-Space Id (0: Kernel's initial)
    state:  usize,  // one of STATE_* | FLAGS_*
    tid:    usize,
}

impl TaskContext {
    pub const STATE_DEAD:       usize = 0; // DEAD or uninitialized
    pub const STATE_NEW:        usize = 1; // Initialized but not run yet
    pub const STATE_READY:      usize = 2; // To be scheduled
    pub const STATE_RUNNING:    usize = 3; // Currently running
    pub const STATE_BLOCKED:    usize = 4; // Waiting for an event (IO, SYNC,..)
    pub const STATE_TERMINATING:usize = 5;


    pub const fn new() -> Self {
        Self {
            pas: 0,
            state: Self::STATE_DEAD,
            ep: empty_task,
            rsp: 0,
            tid: 0,
        }
    }

    pub fn init(&mut self, id: usize, func: fn(), stack: &mut [usize]) {
        let stacklen = stack.len();
        // Initial stack - compatible with the context switch logic in boot.S
        // ----- stack_base -------------------------------
        // ....
        // RSP -> 0x0123456789abcdef "Stack watermark"
        //        0x0 "Default R15"
        //        0x0 "Default R14"
        //        0x0 "Default R13"
        //        0x0 "Default R12"
        //        0x0 "Default R11"
        //        0x0 "Default R10"
        //        0x0 "Default R9"
        //        0x0 "Default R8"
        //        &stack[stacklen-1] " Default RBP"
        //        &task "Default RDI (Parameter for launch_task)""
        //        0x0 "Default RSI"
        //        0x0 "Default RDX"
        //        0x0 "Default RCX"
        //        0x0 "Default RBX"
        //        0x0 "Default RAX"
        //        0x202 "Default RFLAGS value (starts with INT enabled)"
        //        X "Address of TaskContext::launch_task() for the first switch"
        //        0x0 "Return address after launch_task"
        // ---- stack_base + stack_size ------------------- <- &stack[stacklen]
        stack[stacklen - 1] = 0; // Return address after launch_task
        stack[stacklen - 2] = Self::launch_task as *const () as usize;
        stack[stacklen - 3] = 0x202;
        stack[stacklen - 4] = 0; // RAX
        stack[stacklen - 5] = 0; // RBX
        stack[stacklen - 6] = 0; // RCX
        stack[stacklen - 7] = 0; // RDX
        stack[stacklen - 8] = 0; // RSI
        stack[stacklen - 9] = (self as *const TaskContext) as usize; // RDI
        stack[stacklen -10] = (&stack[stacklen-1] as *const usize) as usize; // RBP
        stack[stacklen -11] = 0; // R8
        stack[stacklen -12] = 0; // R9
        stack[stacklen -13] = 0; // R10
        stack[stacklen -14] = 0; // R11
        stack[stacklen -15] = 0; // R12
        stack[stacklen -16] = 0; // R13
        stack[stacklen -17] = 0; // R14
        stack[stacklen -18] = 0; // R15
        stack[stacklen -19] = 0x0123456789abcdef; // Watermark

        self.ep = func;
        self.rsp = (&stack[stacklen - 19] as *const usize) as usize;
        self.state = Self::STATE_NEW;
        self.tid = id;
    }

    pub fn runnable(&self) -> bool {
        self.state >= Self::STATE_NEW && self.state < Self::STATE_BLOCKED
    }

    pub fn tid(&self) -> usize {
        self.tid
    }
    // This function is called as a wrapper of the task's callback to handle
    // the return of the task (i.e., exit)
    fn launch_task(task: &mut TaskContext) {
        // archlog!("Starting task[{}]: state {}, rip:{:X}, rsp:{:X}\n",
        //     task.tid, task.state, task.ep as usize, task.rsp);
        if task.state < Self::STATE_NEW {
            panic!("Starting an uninitialized task!");
        }
        (task.ep)();
        // Terminate the task
        task.state = Self::STATE_DEAD;
        sched::terminate_task();
        panic!("Continued a dead task's code where it have been unreachable!");
    }
}

fn empty_task() {

}

extern "C" {
    fn start_first_thread(task_p: usize);
    fn switch_context(old_p: usize,  new_p: usize);
}

// Per-CPU initialization code calls this for the very first task.
// There is no previous context to retrieve!
pub fn cpu_start_first_task(task: &mut TaskContext) {
    unsafe{
        start_first_thread(task as *mut TaskContext as usize);
    }
}

pub fn cpu_switch_context(from: &mut TaskContext, to: &mut TaskContext) {
    unsafe{
        switch_context(from as *mut TaskContext as usize,
                        to as *mut TaskContext as usize);
    }
}
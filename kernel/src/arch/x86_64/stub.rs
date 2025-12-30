// Rust stub for the x86_64 architecture

use core::mem::size_of;
use core::arch::asm;
use crate::pmm::PMMapElement;
use crate::sched;
use crate::{dump_memory, kstart};

//
// Debugging macros
//
#[cfg(feature="debug_arch")]
use core::fmt::Write;
#[cfg(feature="debug_arch")]
struct ArchDebugConsole;
#[cfg(feature="debug_arch")]
impl Write for ArchDebugConsole {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        kearly_console::print_str(_s.as_bytes());
        Ok(())
    }
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
// Private Data Types                                                        //
//---------------------------------------------------------------------------//
// Multiboot 1 Information
#[repr(C)]
struct MultibootInfo {
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
    color_info: [u8; 6], // Placeholder for actual color info struct/padding
}

#[repr(C, packed)]
struct MemoryEntry {
    size: u32,
    base_addr: u64,
    length: u64,
    mtype: u32,
}

//----------------------------------------------------------------------------//
// Private interface between the Assembly stub (boot.S) and the Rust stub     //
//----------------------------------------------------------------------------//

//
// Kernel's entry-point in Rust
// boot.S must have switched to 64-bit (Long Mode) with the first
// INIT_NUM_PAGE_TABLES*2MB (up to 1GB) of the physical memory directly mapped
// to kernel's initial virtual address space
//
#[unsafe(no_mangle)]
extern "C" fn rust_entry_x86_64(mbi: &MultibootInfo) {
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
    // Todo - fetch VBE's information for a potential graphics driver
    // Todo - Pass a list kernel modules (e.g., ramdisk) Grub loaded for us

    
    kearly_console::init();
    // SMP, LAPIC, IOAPIC, HiRes Event Timer, etc. are found in ACPI tables
    match x86_acpi_parse() {
        Some(acpi) => {
            start_smp(&acpi);
        },
        None => {
            dbg!("No ACPI information found. Multiprocessing disabled.\n");
        }
    };

    // Start the kernel. 
    kstart(&mem_map[0..e820_mmap_count]);
    panic!(); // kstart shouldn't really return but if does, we should panic
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
    panic!("#UD RFLG={:X} CS={:X} RIP={:X} SS={:X} RSP={:X}",
        info.rflg, info.cs, info.rip, info.ss, info.rsp
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
extern "C" fn kexcep_gp_fault() {
    panic!("kexcep_gp_fault");
}

#[unsafe(no_mangle)]
extern "C" fn kexcep_page_fault(exframe: usize) {
    let info = x86_decode_exception_frame(exframe, true);
    dump_memory(info.rsp, 8);
    panic!("#PF CR2={:X} ERR={:X} RFLG={:X} CS={:X} RIP={:X} SS={:X} RSP={:X}",
        info.cr2, info.err, info.rflg, info.cs, info.rip, info.ss, info.rsp
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
    x86_pic::send_eoi(irq); 
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
        let (cr2, cr4): (usize, usize);
        asm!("mov rax, cr2", out("rax")cr2);
        asm!("mov rax, cr4", out("rax")cr4);
        if error_code {
            X86ExceptionInfo {
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

struct X86IoApic;
impl X86IoApic {
    const IOAPIC_REG_ID:    u8 = 0x0;
    const IOAPIC_REG_VER:   u8 = 0x1;

    pub fn read_reg(ioapic: &AcpiIOApic, reg_index: u8) -> u32 {
        let io_reg_sel : *mut u32 = ioapic.ioapic_mmio as *mut u32;
        let io_reg_dat : *mut u32 = (ioapic.ioapic_mmio + 0x10) as *mut u32;
        unsafe {
            *io_reg_sel = reg_index as u32;
            *io_reg_dat
        }
    }

    pub fn write_reg(ioapic: &AcpiIOApic, reg_index: u8, value: u32) {
        let io_reg_sel : *mut u32 = ioapic.ioapic_mmio as *mut u32;
        let io_reg_dat : *mut u32 = (ioapic.ioapic_mmio + 0x10) as *mut u32;
        unsafe {
            *io_reg_sel = reg_index as u32;
            *io_reg_dat = value;
        }
    }

    pub fn init(ioapic: &AcpiIOApic) {
        // Assuming that we're still using the inital kernel address-space...
        dbg!("IOAPIC init => ID = {}\n", 
            Self::read_reg(ioapic, Self::IOAPIC_REG_ID));
    }
}

struct X86LocalApic {

}
//----------------------------------------------------------------------------//
// External interface exposed to kernel's general code                        //
//----------------------------------------------------------------------------//

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
pub fn cpu_read_timestamp() -> u64 {
    let (upper, lower): (u64, u64);
    unsafe {
        asm!("rdtsc", out("rdx")upper, out("rax")lower);
    }
    (upper << 32) | lower
}
pub fn cpu_busywait(delay_tsc: u64) {
    let target_tsc = cpu_read_timestamp() + delay_tsc;
    while cpu_read_timestamp() < target_tsc {}
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

//
// Initial/Boot-time Console
// VGA/80x24TXT mode
//
pub mod kearly_console {
    use core::sync::atomic::*;

    const VGA_BASE: u32 = 0xb8000;
    static VGA_CUR : AtomicUsize = AtomicUsize::new(0);

    pub fn init() {
        let fill : u16 = 0x1f00 | b' ' as u16;
        let mut ptr: *mut u16 = VGA_BASE as *mut u16;
        for _ in 0..80*25 {
            unsafe { *ptr = fill; }
            ptr = ptr.wrapping_add(1);
        }
    }

    pub fn print_str(msg: &[u8]) {
        let color_byte: u16 = 0x1f00;
        let mut ptr: *mut u16 = VGA_BASE as *mut u16;
        let mut cursor = VGA_CUR.load(Ordering::Relaxed);
        ptr = ptr.wrapping_add(cursor);
        for &c in msg {
            if c == b'\n' {
                if (cursor / 80) < 24 {
                    // Move the cursor to the next line
                    cursor =(cursor / 80 + 1) *  80;
                } else {
                    cursor = 0; // wrap around
                }
                ptr = (VGA_BASE as *mut u16).wrapping_add(cursor);
            } else {
                unsafe {*ptr = c as u16 | color_byte;}
                ptr = ptr.wrapping_add(1);
                cursor = (cursor + 1) % (80 * 25);
            }
        }
        VGA_CUR.store(cursor, Ordering::Relaxed);
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
fn start_smp(acpi: &AcpiInfo) {
    // Print what we found on ACPI tables if compiled with debug_arch
    // CPUs/LAPICS
    dbg!("LAPIC MMIO @ {:X}\n", acpi.lapic_mmio);
    for _i in 0..acpi.lapic_cnt as usize {
        dbg!("CPU[{}]: LAPIC ID: {}, Enabled: {}\n",
            acpi.lapic[_i].cpu_id,
            acpi.lapic[_i].lapic_id,
            acpi.lapic[_i].enabled
        );
    }
    // IOAPIC
    dbg!("IOAPIC[{}]: MMIO Base: {:X}, GSI Base: {:X}\n",
        acpi.ioapic.ioapic_id,
        acpi.ioapic.ioapic_mmio,
        acpi.ioapic.gsi_base
    );
    // IRQ->GSI mappings
    for _i in 0..acpi.irq_map_cnt as usize {
        dbg!("<IRQ#{}.{} -> GSI#{} ON {}{}> ",
            acpi.irq_map[_i].src_bus,
            acpi.irq_map[_i].src_irq,
            acpi.irq_map[_i].dst_gsi,
            if acpi.irq_map[_i].active_low {"Low"} else {"High"},
            if acpi.irq_map[_i].lvl_trig {"Level"} else {"Edge"}
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
    // Initialize IOAPIC: TODO MOVE TO GENERAL CODE SOMEHOW
    X86IoApic::init(&acpi.ioapic);

}

//
// System Configuration (ACPI)
//
pub const MAX_CPU_COUNT: usize = 8;
pub const MAX_IRQ_COUNT: usize = 16;

#[derive(Default)]
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

#[derive(Default)]
struct AcpiLocalApic {
    cpu_id:     u8,
    lapic_id:   u8,
    enabled:    bool
}

#[derive(Default)]
struct AcpiIOApic {
    ioapic_id:  u8,
    ioapic_mmio:u32,
    gsi_base:   u32 // must be 0 in a single IOAPIC config
}

#[derive(Default)]
struct AcpiIRQMapping {
    src_bus:    u8,
    src_irq:    u8,
    dst_gsi:    u32,
    active_low: bool, // Active Low or Active High signal
    lvl_trig:   bool, // Triggered on the Level or on the Edge of the signal
}

#[derive(Default)]
struct AcpiNmiMapping {
    cpu_id_mask:u8,
    lint_vector:u8, // Entry# of the vector table of CPUs' LAPIC
    active_low: bool,
    lvl_trig:   bool
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
    let mut ret = AcpiInfo::default();
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

// Doesn't seem to be supported any more - at least on QEMU it only shows 1 cpu
// fn x86_parse_mp_config() {
//     // 1) Finding the MP Floating Pointer Structure left for us by BIOS
//     //    Look for the signature  "_MP_" or 0x5F504D5F in the first MB of
//     //    the memory. TODO add support for ACPI and find it via ACPI!
//     // MPFP FORMAT:
//     // signature:  u32,
//     // config_tlb: u32,
//     // length:     u8, // multiplied by 16 bytes
//     // mp_rev:     u8, // MP Spec Revision
//     // checksum:   u8, // added to sum of all other bytes of this struct -> 0
//     // def_config: u8, // must be zero otherwise find the default cfg
//     // features:   u32 // Bit 7 set: IMCR + and PIC mode, virt. wire mode otherwise
//     //
//     // For now I assume that features=0 and there is no default config.
//     let mut ptr: *mut u32 = 0x400 as *mut u32;
//     let mut valid_mpfp = false;

//     for _i in 0..0x40000 {
//         unsafe {
//             if *ptr == 0x5F504D5F {
//                 // check the sum
//                 let mut sump: *mut u8 = ptr as *mut u8;
//                 let mut sum: u8 = 0;
                
//                 for _j in 0..16 {
//                     sum += *sump;
//                     sump = sump.wrapping_add(1);
//                 }
//                 if sum == 0 {
//                     // Valid MPFP!
//                     valid_mpfp = true;
//                     break;
//                 }
//             }
//             ptr = ptr.wrapping_add(1);
//         }
//     }
//     // 2) Read the MP Config Table MPFP points to
//     // MPCT FORMAT:
//     // signature: u32 // "PCMP" = 0x504D4350
//     // len: u16
//     // mp_rev: u8
//     // checksum: u8
//     // oem_id: u64;
//     // prod_id: [u8; 12];
//     // oem_table: u32
//     // oem_table_size: u16
//     // entry_count: u16; // #of CPU/IOAPIC entries after this struct (offset 34)
//     // lapic_address: u32// MMIO base of the local APICs (offset 36): FEE00000
//     // extended_table_length: u16
//     // extended_table_checksum: u8;
//     // rsvd: u8;
//     if !valid_mpfp {
//         archlog!("No SMP support.\n");
//         return;
//     }
//     unsafe {
//         archlog!("MPFP @{:p}, ConfTlb:{:x}, lrcd:{:X}, feat:{:X}\n",
//             ptr, *(ptr.wrapping_add(1)), 
//             *(ptr.wrapping_add(2)), *(ptr.wrapping_add(3))
//         );
//         // Make ptr point to MPConfig
//         let ptr: *mut u32 = *(ptr.wrapping_add(1)) as *mut u32;
//         if (*ptr) !=  0x504D4350 {
//             archlog!("No valid MP Configuration was found\n");
//             return;
//         }
//         let entry_cnt: u16 = *((ptr as *mut u8).wrapping_add(34) as *mut u16);
//         let lapic_adr: u32 = *((ptr as *mut u8).wrapping_add(36) as *mut u32);
//         archlog!("#entries: {}, LAPIC_BASE: {:X}\n", entry_cnt, lapic_adr);
//         // 3) Iterate over entries and find CPUs and IOAPICs
//         let mut entry: *mut u32 = ptr.wrapping_add(11);
//         for _ in 0..entry_cnt {
//             let ent_type : u8 = ((*entry) & 0xFF) as u8;
//             let id  = ((*entry) & 0xFF00) >> 8 as u8;
//             let ver = ((*entry) & 0xFF0000) >> 16 as u8;
//             let flg = ((*entry) & 0xFF000000) >> 24 as u8;
//             if ent_type == 0 {
//                 archlog!("CPU[{}]: LAPIC Version: {}, Flags: {:X}\n",
//                         id, ver, flg);
//                 entry = entry.wrapping_add(5); // CPU entries are 20 bytes
//             } else if ent_type == 2 {
//                 let ioapic_adr : u32 = *(entry.wrapping_add(1));
//                 archlog!("IOAPIC[{}]: Version: {}, Flags: {:X}, Addr: {:X}\n",
//                         id, ver, flg, ioapic_adr);
//                 entry = entry.wrapping_add(2); // IOAPIC entries are 8 bytes
//             } else {
//                 //archlog!("-- ENT TYPE: {} -- ", ent_type);
//                 entry = entry.wrapping_add(2); // Other entries are 8 bytes
//             }
//         }
//     }
// }

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
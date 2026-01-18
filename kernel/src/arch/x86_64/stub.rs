// 
// Rust stub for the x86_64 architecture
//
#![allow(dead_code)]

use core::arch::asm;
use crate::arch::asc::vga::*;
use crate::mem::physical::{PMMapElement, palloc};
use crate::sched::Task;
use crate::{Syscall, SyscallHandlerFn, SyscallOpCode, util::*};
use crate::{dump_memory, kstart};
use core::fmt::Write;

mod vga;

//
// Debugging macros
//

// Serial Port Debugging
#[cfg(feature="debug_arch")]
macro_rules! dbg {
    ($($arg:tt)*) => {
        let mut debug_console = DebugOut;
        let _ = write!(&mut debug_console, "[X64] ");
        let _ = write!(&mut debug_console, $($arg)*);
    };
}
#[cfg(not(feature="debug_arch"))]
macro_rules! dbg{
    ($($arg:tt)*) => { };
}


//---------------------------------------------------------------------------//
// Private Data Types and Globals                                            //
//---------------------------------------------------------------------------//
// Multiboot 2 Information
#[repr(u32)]
#[derive(Clone, Copy)]
enum Mulitboot2TagType {
    End             = 0,
    CmdLine         = 1,
    BootLoaderName  = 2,
    Module          = 3,
    BasicMemInfo    = 4,
    BootDevice      = 5,
    MemoryMap       = 6,
    VBE             = 7,
    FrameBuffer     = 8,
    ElfSections     = 9,
    APM             = 10,
    EFI32           = 11,
    EFI64           = 12,
    SMBIOS          = 13,
    ACPIOld         = 14,
    ACPINew         = 15,
    Network         = 16,
    EFIMemMap       = 17,
    EFIBS           = 18,
    EFI32IH         = 19,
    EFI64IH         = 20,
    LoadBaseAddr    = 21
}
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Multiboot2TagModule{
    mod_start: u32,
    mod_end:   u32,
    cmd_line:  [u8; 0]
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Multiboot2TagBasicMemInfo{
    mem_lower: u32,
    mem_upper:   u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Multiboot2MemoryMapEntry{
    addr:       u64,
    len:        u64,
    mtype:      u32, // 1: Available, 2: RSVD, 3: ACPI_REC, 4: NVS, 5: BADRAM
    zero:       u32
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Multiboot2MemoryMap{
    entry_size:     u32,
    entry_version:  u32,
    entries:        [Multiboot2MemoryMapEntry; 0]
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Multiboot2VBE{
    mode:               u16,
    interface_seg:      u16,
    interface_off:      u16,
    interface_len:      u16,
    info_block:         [u8; 512],
    mode_info_block:    [u8; 512]
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Multiboot2FrameBuffer {
    addr:               u64,
    pitch:              u32,
    width:              u32,
    height:             u32,
    bpp:                u8,
    fbtype:             u8, // 0 INDEXED, 1 RGB, 2 EGA_TEXT
    rsvd:               u16,
    color_info:         [u8; 6],
}

union Multiboot2TagData {
    module :                Multiboot2TagModule,
    basic_mem_info:         Multiboot2TagBasicMemInfo,
    mem_map:                Multiboot2MemoryMap, 
    vbe_info:               Multiboot2VBE,
    frame_buffer:           Multiboot2FrameBuffer,
    efi32_pointer:          u32,
    efi64_pointer:          u64,
    acpi_old_rsdp:          [u8; 20],
    acpi_new_xsdp:          [u8; 36]

}

#[repr(C, packed)]
struct Multiboot2Tag {
    ttype:   Mulitboot2TagType,
    tsize:   u32,
    tdata:   Multiboot2TagData
}

pub struct MachineContext {
    cpu_count:  usize,
    acpi_info:  AcpiInfo,
    ioapic:     X86IoApic,
}

impl MachineContext {
    pub const fn new() -> Self {
        Self {
            cpu_count:  1, // There's at least one cpu (BSP), lol!
            acpi_info:  AcpiInfo::new(),
            ioapic:     X86IoApic::new(),
        }
    }
}

static THIS_MACHINE: Spinlock<MachineContext> = 
    Spinlock::new(MachineContext::new());
static VESA_CONTEXT: Spinlock<VESAContext> = Spinlock::new(VESAContext::new());

percpu_global! {
    THIS_PERCPU_BASE: usize = 0; // To avoid rdmsr(IA32_GS_BASE) every time
    THIS_LAPIC:  X86LocalApic = X86LocalApic::new();
    THIS_IOAPIC: X86IoApic    = X86IoApic::new();
    THIS_TSC:    X86TimeStampCounter = X86TimeStampCounter::new();
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
extern "C" fn rust_x864_entry_bsp(mb2info_base: usize, max_cpus: usize) {
    // Physical Memory Map Buffer
    let mut mem_map: [PMMapElement; 32] = [
        PMMapElement {base: 0, len: 0, avail: false}; 32];
    let mut mem_map_count = 0;
    let mut acpi_rsdp: Option<usize> = None;

    // Init user program (module) addr:
    let mut mod_base: usize = 0;
    let mut mod_end: usize = 0;
    // Enumerate Muliboot 2 tags
    // Use the default serial port for debugging since VESA is not yet enabled
    PortBasedUART::new(0x3F8).config();
    dbg!("MULTIBOOT2 BASE: {:X}\n", mb2info_base);
    let mut total_size: usize;
    let mut tag_base: *mut u8 = (mb2info_base + 8) as *mut u8;
    unsafe {
        total_size = *(mb2info_base as *mut u32) as usize;
    }
    dbg!("TOTAL SIZE: {}\n",total_size);
    while total_size > 0 {
        let tag: *mut Multiboot2Tag = tag_base as *mut Multiboot2Tag;
        let tag_size: usize;
        let tag_type: Mulitboot2TagType;
        unsafe {
            tag_size = (*tag).tsize as usize;
            tag_type = (*tag).ttype;
        }
        let tag_pad  = tag_base.wrapping_add(tag_size).align_offset(8);
        dbg!("TAG TYPE: {} SIZE: {} (Left: {})\n",
                    tag_type as u32, tag_size, total_size);
        match tag_type {
            Mulitboot2TagType::ACPIOld          => {
                // ACPI REV 1 (RSDP -> SDT w\ 32-bit table addresses)
                acpi_rsdp = Some((tag as usize) + 8);
            }
            Mulitboot2TagType::ACPINew          => {
                // ACPI REV 2+ (XSDP -> SDT w\ 64-bit table addresses)
                acpi_rsdp = Some((tag as usize) + 8);
            }
            Mulitboot2TagType::FrameBuffer      => {
                let mut vesa = VESA_CONTEXT.lock();
                unsafe{
                    (*vesa).init_from_mb2(None, 
                                         Some(&((*tag).tdata.frame_buffer)));
                }
            },
            Mulitboot2TagType::VBE              => {
                let mut vesa = VESA_CONTEXT.lock();
                unsafe {
                    (*vesa).init_from_mb2(Some(&((*tag).tdata.vbe_info)), None);
                }
            },
            Mulitboot2TagType::MemoryMap        => {
                let ent_size : usize;
                unsafe {
                    ent_size = (*tag).tdata.mem_map.entry_size as usize;
                } 
                mem_map_count = (tag_size - 8) / ent_size;
                dbg!("MEMMAP - ENT_SIZE: {}, COUNT: {}\n", 
                        ent_size, mem_map_count);
                let mut ent = (tag as usize + 16) as *mut Multiboot2MemoryMapEntry;
                for i in 0..32 {
                    if i >= mem_map_count {
                        break;
                    }
                    unsafe {
                        mem_map[i].base = (*ent).addr as usize;
                        mem_map[i].len  = (*ent).len as usize;
                        // Mark any available memory < 1MB as reserved
                        if mem_map[i].base < 0x100000 {
                            mem_map[i].avail = false;
                        } else {
                            mem_map[i].avail = match (*ent).mtype {
                                1 => true,
                                _ => false,
                            };
                        }
                    }
                    ent = ent.wrapping_add(1);
                }

            }
            Mulitboot2TagType::Module           => {
                unsafe {
                    mod_base    = (*tag).tdata.module.mod_start as usize;
                    mod_end     = (*tag).tdata.module.mod_end as usize;
                    dbg!("Init Program loaded @ {:X} to {:X}\n",
                        mod_base, mod_end);
                }
                
            }
            Mulitboot2TagType::End              => {
                break;
            }
            _                                   => {}
        }
        tag_base = tag_base.wrapping_add(tag_size + tag_pad);
        total_size -= tag_size + tag_pad;
    }
    
    // Set a default video mode, and 
    // Initialize the early-stage standard output (clears the screen too)
    let (rows, cols, mode): (u32, u32, u16);
    {
        let mut vesa = VESA_CONTEXT.lock();
        (*vesa).set_background_rgb((240, 240, 240));
        (*vesa).set_foreground_rgb((0, 0, 255));
        (rows, cols) = (*vesa).screen_size();
        mode = (*vesa).mode_number();
    }
    kearly_console::init();
    klog!("VESA Graphics: Mode=0x{:X}, Rows:{}, Columns:{}\n", mode, rows, cols);

    // Todo - fetch kernel's boot command-line/parameters
    // Todo - Pass a list kernel modules (e.g., ramdisk) Grub loaded for us
    
    // Initialize the per-cpu sections
    let bsp_cpu_id = cpu_id(); // LAPIC ID
    percpu_init_sections();
    percpu_init_cpu(bsp_cpu_id);
    *THIS_CPU_ID.borrow_mut() = bsp_cpu_id;
    THIS_TSC.borrow_mut().init();
    
    // SMP, LAPIC, IOAPIC, HiRes Event Timer, etc. are found in ACPI tables

    match x86_acpi_parse(acpi_rsdp) {
        Some(acpi) => {
            {
                // No concurrency here, but Rust!
                let mut this_machine = THIS_MACHINE.lock();
                (*this_machine).acpi_info = acpi;
            }
            // Start the application processors
            start_smp(bsp_cpu_id, max_cpus);
        },
        None => {
            dbg!("No ACPI information found. Multiprocessing disabled.\n");
        }
    };

    // Fix the syscall interrupt entry (0x20) in IDT to accept calls from Ring3
    extern "C" {
        static idt_base : usize;
    }
    unsafe {
        let idte: *mut u64 = &idt_base as *const usize as *mut u64;
        *(idte.wrapping_add(0x20 * 2)) |= 0x600000000000; // DPL = 3
    }

    // Start the kernel.
    if mod_base == 0 {
        // No RAMDISK
        kstart(bsp_cpu_id, Some(&mem_map[0..mem_map_count]), None );
    } else {
        kstart(bsp_cpu_id, Some(&mem_map[0..mem_map_count]),
                Some(crate::RamdiskInfo {
                    start_phy_addr: mod_base,
                    length: mod_end - mod_base + 1
                })
        );
    }
    panic!(); // kstart shouldn't really return but if does, we should panic
}

#[unsafe(no_mangle)]
extern "C" fn rust_x864_entry_ap(_arg: usize) {
    let cpuid = cpu_id();
    percpu_init_cpu(cpuid);
    THIS_CPU_ID.write(cpuid);
    THIS_TSC.borrow_mut().init();
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
    kstart(cpuid as usize, None, None);
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
extern "C" fn kirq_lvt_timer() {
    THIS_LAPIC.borrow_mut().send_eoi();
    // Set TSC_DEADLINE if periodic mode is selected
    unsafe {
        X86_LAPIC_TIMER_HANDLER(*THIS_CPU_ID.borrow() as u16);
    }
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
    panic!("Stack Corruption (Context Switch). CPU={}",
        *(THIS_CPU_ID.borrow()));
}


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
        x86_ioport_write(PIT_PORT_CMD, cmd);
        x86_ioport_write(PIT_PORT_CH0, (reload & 0xFF) as u8);
        x86_ioport_write(PIT_PORT_CH0, (reload >> 8)   as u8);
    }

    pub fn config_oneshot_count(ms: u32) {
        // Use Channel 2 in One-Shot mode to count down to zero
        
        // Since CH2 is wired to the PC speaker, the gated-output of the speaker
        // should be disabled first (see bits 0-1 of port 0x61, SysControlPortB)
        let sys_ctl_b: u8 = x86_ioport_read(0x61);
        x86_ioport_write(0x61, sys_ctl_b & 0xFC);
        // 
        let reload : u16 = (PIT_FREQ_KHZ * ms as f64) as u16;
        let cmd = make_cmd(2, PIT_ACCESS_LOW_HI, PIT_OPMODE_ONESHOT);
        x86_ioport_write(PIT_PORT_CMD, cmd);
        x86_ioport_write(PIT_PORT_CH0, (reload & 0xFF) as u8);
        x86_ioport_write(PIT_PORT_CH0, (reload >> 8)   as u8);
    }

    // Should be called after config_oneshot_sleep is called
    pub fn start_oneshot_count(){
        // Clear and then reset bit 0 of IO port 0x61, after modifying the
        // reload value, hence, start counting down.
        let sys_ctl_b: u8 = x86_ioport_read(0x61);
        x86_ioport_write(0x61, sys_ctl_b & 0xFE); // Clear bit 0
        x86_ioport_write(0x61, sys_ctl_b | 0x01); // Set bit 0
    }

    pub fn wait_for_oneshot_count() {
        // bit 5 of port 0x61 will go high once the counter hits zero
        while x86_ioport_read(0x61) & 0x20 == 0 {
        }
    }
}


// For newer CPUs that support Invariant TSCs, this is going to be used
// for time-keeping and preemption interrupts
// TODO: fallback to PIT (or HPET) in case the system doesn't support this
struct X86TimeStampCounter {
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
                    dbg!("INVARIANT TSC FREQ: {} - EAX: {} EBX {} ECX: {}\n",
                            self.freq_hz, eax, ebx, ecx);

                    // Also approximate it:
                    // x86_pit::config_oneshot_count(100); // 100 ms count-down
                    // x86_pit::start_oneshot_count();
                    // let start_tsc = cpu_read_timestamp();
                    // x86_pit::wait_for_oneshot_count();
                    // let end_tsc = cpu_read_timestamp();
                    // let tsc_overhead1 = cpu_read_timestamp();
                    // log!("INVARIANT TSC FREQ APPROX ~ {} HZ\n", 
                    //     (end_tsc - start_tsc - (tsc_overhead1-end_tsc)*2) * 10);
                } else {
                    // Use PIT to approximate it
                    x86_pit::config_oneshot_count(100); // 100 ms count-down
                    x86_pit::start_oneshot_count();
                    let start_tsc = cpu_read_timestamp();
                    x86_pit::wait_for_oneshot_count();
                    let end_tsc = cpu_read_timestamp();
                    let tsc_overhead1 = cpu_read_timestamp();
                    self.freq_hz = (end_tsc - start_tsc - (tsc_overhead1-end_tsc)*2) * 10;
                    self.enabled = true;
                    dbg!("INVARIANT TSC FREQ APPROX: {} HZ\n", self.freq_hz);
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

    const MSR_IA32_TSC_DEADLINE:u32 = 0x6E0;

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
    
    //
    // IRQ Functionalities
    //

    pub fn send_eoi(&mut self) {
        self.write_reg(Self::REG_EOI, 0);
    }
    

    //
    // Inter-Processor Communication
    //

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
		cpu_busywait_us(200);
		self.wait_ipi_send();
    }

    //
    // Timer Functionalities
    // For now only One-shot TSC_DEADLINE mode is supported
    // See section 12.5.4.1 TSC-Deadline Mode in Intel's SDM
    //
    // REG_LVT_TIMER bit specification:
    // IRQ Handler Vector:              [7..0]
    // Delivery Status                  [12]
    // IRQ Masked                       [16]
    // Timer Mode                       [18..17]
    pub fn config_timer(&mut self, irq_handler_vector: u8, irq_masked: bool) {
        self.write_reg(Self::REG_ERROR_STATUS, 0);
        // Set the initial tsc deadline to 0 so that no unwanted IRQ is raised
        self.set_timer(0);
        // Set the mode to TSC_DEADLINE (LVT_TIMER.bits[18..17] <- 10b)
        // Set the vector to irq_handler_vector
        // unmask the IRQ if necessary
        let mut lvtreg = self.read_reg(Self::REG_LVT_TIMER) & 0xFFF8EF00;
        if irq_masked{
            lvtreg |= 1 << 16;
        }
        lvtreg |= irq_handler_vector as u32 | (1 << 18);
        self.write_reg(Self::REG_LVT_TIMER, lvtreg);
    }

    //  Writing a non-zero 64-bit value into IA32_TSC_DEADLINE arms the timer.
    pub fn set_timer(&mut self, target_tsc: u64) {
        x86_msr_write(Self::MSR_IA32_TSC_DEADLINE, target_tsc);
    }

    pub fn set_timer_irq_mask(&mut self, irq_masked: bool) {
        let mut lvtreg = self.read_reg(Self::REG_LVT_TIMER) & 0xFFFEFFFF;
        if irq_masked{
            lvtreg |= 1 << 16;
        }
        self.write_reg(Self::REG_LVT_TIMER, lvtreg);
    }
}

//----------------------------------------------------------------------------//
// External interface exposed to kernel's general code                        //
//----------------------------------------------------------------------------//
percpu_global!{
    pub THIS_CPU_ID: usize = 0; // To avoid issuing cpuid every time
    pub THIS_CPU_SYSTIMER:  SystemTimer = SystemTimer::new();
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
pub fn cpu_ints_enabled() -> bool {
    let rflg = x64_read_rflags();
    rflg & 0x200 > 0
}
pub fn cpu_unmask_irq(irq: u32) {
    THIS_IOAPIC.borrow_mut().set_irq_mask(irq, false);
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
    let apic_cpu_id: u32;
    (_, _, _, apic_cpu_id) = x86_cpuid_inst(0xB, 0x0);
    apic_cpu_id as usize
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
pub fn cpu_busywait_us(delay_us: u64) {
    let tsc = THIS_TSC.borrow_mut();
    let mut freq_hz = 1_500_000_000; // Default to 1.5GHz by default
    if tsc.enabled {
        freq_hz = tsc.freq_hz;
    }
    let delay_tsc = freq_hz / 1_000_000 * delay_us;
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

pub fn x64_read_rflags() -> u64 {
    let r: u64;
    unsafe {
        asm!("pushfq", "pop {}", out(reg) r, options(nomem, preserves_flags));
    }
    r
}

unsafe extern "C" {
    static tss64_base: usize;
}
pub fn x64_tss_rsp0_addr() -> usize{
    let cpuid = *(THIS_CPU_ID.borrow());
    
    unsafe{
        let base = &tss64_base as *const usize as usize;
        base + (104 * cpuid) +4
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

pub struct PortBasedUART {
    port:   u16,
}
impl PortBasedUART {
    pub const fn new(p: u16) -> Self {
        Self {
            port: p,
        }
    }

    pub fn config(&mut self) {
        x86_ioport_write(self.port + 1, 0x00);    // Disable all interrupts
        x86_ioport_write(self.port + 3, 0x80);    // Enable DLAB (set baud rate divisor)
        x86_ioport_write(self.port + 0, 0x03);    // Set divisor to 3 (lo byte) 38400 baud
        x86_ioport_write(self.port + 1, 0x00);    //                  (hi byte)
        x86_ioport_write(self.port + 3, 0x03);    // 8 bits, no parity, one stop bit
        x86_ioport_write(self.port + 2, 0xC7);    // Enable FIFO, clear them, with 14-byte threshold
        x86_ioport_write(self.port + 4, 0x0B);    // IRQs enabled, RTS/DSR set
        x86_ioport_write(self.port + 4, 0x1E);    // Set in loopback mode, test the serial chip
        x86_ioport_write(self.port + 0, 0xAE);    // Test serial chip (send byte 0xAE and check if serial returns same byte)
        // If serial is not faulty set it in normal operation mode
        // (not-loopback with IRQs enabled and OUT#1 and OUT#2 bits enabled)
        x86_ioport_write(self.port + 4, 0x0F);


    }

    pub fn putc(&self, c: u8) {
        while x86_ioport_read(self.port + 5) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        x86_ioport_write(self.port, c);
    }

    pub fn puts(&self, msg: &[u8]) {
        for &c in msg {
            self.putc(c);
        }
    }

}

pub mod kdebug_console {
    use crate::arch::PortBasedUART;
    pub fn print_str(msg: &[u8]) {
        let uart = PortBasedUART::new(0x3F8);
        uart.puts(msg);
    }
}

//
// Initial/Boot-time Console
// VGA/80x24TXT mode
// Provides a print_str to klog! or similar debug/log printing routines.
// print_str is implemented in a synchronizing manner
//
pub mod kearly_console {
    use crate::arch::asc::*;
    use crate::util::Spinlock;

    static CURSOR : Spinlock<(u32, u32)> = Spinlock::new((0,0));
    // CURSOR.0 -> row, .1 -> column

    pub fn init() {
        let mut vesa = VESA_CONTEXT.lock();
        (*vesa).clean_screen();
    }


    pub fn print_str(msg: &[u8]) {
        let (sh, sw) : (u32, u32);
        let (fh, fw) : (u32, u32);
        {
            let vesa = VESA_CONTEXT.lock();
            (sh, sw) = (*vesa).screen_size();
            (fh, fw) = (*vesa).font_size();
        }
        let (rows, cols) = (sh / fh, sw / fw);
        let mut cursor = CURSOR.lock();
        for &c in msg {
            if c == b'\n' {
                (*cursor).0 = ((*cursor).0 + 1) % rows;
                (*cursor).1 = 0;
            } else {
                {
                    let mut vesa = VESA_CONTEXT.lock();
                    (*vesa).putc(c, (*cursor).0, (*cursor).1);
                }
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
#[derive(Clone, Copy, Debug)]
pub enum SysTimerDuration {
    Seconds(u64),
    Milliseconds(u64),
    Microseconds(u64),
    Nanoseconds(u64),
    Ticks(u64)
}

pub enum SysTimerMode {
    OneShot,
    Periodic,
    Disabled
}

// System-wide - Every core runs the same IRQ handler function


pub struct SystemTimer {
    mode: SysTimerMode
}
impl SystemTimer {
    pub const fn new() -> Self {
        Self{
            mode: SysTimerMode::Disabled
        }
    }

    // To be called once during kernel's serialized initialization to install a
    // single IRQ handler. Every core will execute the same handler code, even
    // though each having an individual timer (and set of events)
    pub fn global_init(isr_callback: IsrHandlerFn) {
        unsafe {
            X86_LAPIC_TIMER_HANDLER = isr_callback;
        }
    }

    // Per-CPU - Each CPU can configure a different mode for its timer
    pub fn set_mode(&mut self, mode: SysTimerMode){
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
        self.mode = mode;   
    }

    // Per-CPU - Sets the period of IRQs or the next IRQ to generate depending
    // on the mode set for the timer.
    pub fn arm(&self, duration: SysTimerDuration) {
        match self.mode {
            SysTimerMode::Disabled      => {},
            SysTimerMode::OneShot       => {self.arm_one_shot(duration);}
            SysTimerMode::Periodic      => {self.arm_periodic(duration);}
        }
    }

    //
    fn arm_one_shot(&self, d: SysTimerDuration) {
        let tsc = THIS_TSC.borrow_mut();
        let duration_tsc = match d {
            SysTimerDuration::Ticks(t)          => {
                t
            },
            SysTimerDuration::Seconds(s)        => {
                s * tsc.freq_hz
            },
            SysTimerDuration::Milliseconds(ms)  => {
                ((ms as f64 / 1_000.0) * tsc.freq_hz as f64) as u64
            },
            SysTimerDuration::Microseconds(us)  => {
                ((us as f64 / 1_000_000.0) * tsc.freq_hz as f64) as u64
            },
            SysTimerDuration::Nanoseconds(ns)   => {
                ((ns as f64 / 1_000_000_000.0) * tsc.freq_hz as f64) as u64
            },
        };
        let target = tsc.read() + duration_tsc;
        THIS_LAPIC.borrow_mut().set_timer(target);
    }

    fn arm_periodic(&self, _p: SysTimerDuration) {
        panic!("Not implemented yet!\n");
    }
}



//
// IRQ Interface
//
type IsrHandlerFn = fn(u16);

fn isr_default_imp(_: u16) { }
static mut X86_LAPIC_TIMER_HANDLER: IsrHandlerFn = isr_default_imp;
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

pub fn cpu_trigger_systimer_irq() {
    unsafe{
        asm!(
            "int 0x21" // See boot.S
        );
    };
}


//
// Process Address Space and MMU 
//

pub struct Process {
    pml4_base: usize,
}
impl Process {
    // See boot.S for our GDT Entries
    const GDTE_USER_CODE: u16 = 0x18;
    const GDTE_USER_DATA: u16 = 0x20;
    // Segment Selector Values: See Section 3.4.2 - Segment Selectors
    const SEGSEL_USER_CODE: u16 = Self::GDTE_USER_CODE | 0x3; // CPL: Ring 3
    const SEGSEL_USER_DATA: u16 = Self::GDTE_USER_DATA | 0x3; // CPL: Ring 3
    // Paging Structure Entry Definitions
    const PGENT_PRESENT:        u64 = 0x1;
    const PGENT_WRITABLE:       u64 = 0x2;
    const PGENT_USERMODE:       u64 = 0x4;
    const PGENT_PWT:            u64 = 0x8;  // Page-level Write-throuhg
    const PGENT_PCD:            u64 = 0x10; // Page-level Cache Disable
    const PGENT_PS:             u64 = 0x80; // Set for large pages
    const PGENT_G:              u64 = 0x100; // Global
    
    pub const fn new() -> Self {
        Self {
            pml4_base: 0
        }
    }

    // Creates the initial paging structures for the process that includes
    // the kernel mappings. The rest should be
    pub fn init(&mut self) {
        // Allocate and zero out:
        //   1 page for the PML4 table and 1 page for the first PDPT table
        self.pml4_base  = palloc().expect("Out of memory");
        let pdpt0       = palloc().expect("Out of memory");
        unsafe {
            raw_memset(self.pml4_base, 4096, 0);
            raw_memset(pdpt0,          4096, 0);
        }

        // Set PML4[0] --> PDPT0 that covers the first 512 GB
        let pml4e0 = pdpt0 as u64 | Self::PGENT_PRESENT |
                    Self::PGENT_WRITABLE | Self::PGENT_USERMODE;
        Self::write_table_entry(self.pml4_base, 0, pml4e0);

        // Set PDPT0[0..=3] to Identity-map the first 4GB as user-mode for now
        let pdpt0e = Self::PGENT_PRESENT | Self::PGENT_WRITABLE |
                    Self::PGENT_USERMODE | Self::PGENT_PS; // 1GB page
        for i in 0..4 {
            let phys_addr : u64 = i << 30;
            Self::write_table_entry(pdpt0, i as usize, pdpt0e | phys_addr);
        }

        // Log everything for testing
        dbg!("PML4 Base: {:X}, PML4E0: {:X}\n",
            self.pml4_base,
            Self::read_table_entry(self.pml4_base, 0)
        );
        dbg!("PDPT0 Base: {:X}\n", pdpt0);
        for _i in 0..4 {
            dbg!("    PDPT0[{}] : {:X}\n", _i,
                    Self::read_table_entry(pdpt0, _i));
        }
    }

    /*
     * Execution/Segmentation Management methods
     */
    ///
    /// Converts the currently running kernel task into a user-space task as a
    /// part of this process address space. The calling (kernel) task will not
    /// return to the next instruction after its call to move_to_userspace.
    /// The user-space execution must end with an Exit system call, at which
    /// point the task terminates.
    /// 
    pub fn move_to_userspace(&self, entry_point: usize, user_stack: usize) {
        // Prepare CS, DS, SS for ring 3 transition and then jump to the
        // entry point address given. x64 doesn't support ljmp, so Iretq it is!
        // Should save the RSP0 pointer in the TSS for this CPU so that when
        // the cpu traps in ring-0 again, kernel's stack is recovered
        let _tss_rsp0: usize = x64_tss_rsp0_addr();
        dbg!("TSS[0].RSP0 is located at {:X}\n", _tss_rsp0);
        unsafe {
            switch_to_userspace(entry_point, user_stack, self.pml4_base,
                                x64_tss_rsp0_addr());
        }
        panic!("Must have been unreachable!\n");
    }

    //
    // Paging structure management methods 
    //
    // Virtual Address ----> Physical Address translation
    // 4GB - 0         ----> 4GB - 0 as four 1GB pages as supervisor access
    // Above 4GB       ----> Non-contiguous 4KB physical pages as user access
    //
    //
    pub fn page_size() -> usize {
        0x1000
    }

    pub fn addr_to_page_index(addr: usize) -> usize {
        addr >> 12
    }

    // Maps a page (virtual address > 4GB) to a frame (physical address) 
    pub fn map_pages(_virt_addr: usize, _phys_addr: usize, _num_pages: usize,
                    _privileged: bool, _writeable: bool, _executable: bool,
                    _caching: MmuCachingPolicy) {
        // Todo - Should look at CR3, map the structs into a temporary scratchpad
        //        space and then add the new map.
        // Forgot how MTRR registers affect specific memory region caching...
        // Look it up.
    }

    pub fn unmap_pages(_virt_addr: usize, _num_pages: usize) {

    }

    fn write_table_entry(table_virt_base: usize, index: usize, value: u64) {
        unsafe {
            let destp : *mut u64 = table_virt_base as *mut u64;
            *(destp.wrapping_add(index)) = value;
        }
    }

    fn read_table_entry(table_virt_base: usize, index: usize) -> u64 {
        unsafe {
            let destp : *mut u64 = table_virt_base as *mut u64;
            *(destp.wrapping_add(index))
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Release the paging structures
    }
}

pub enum MmuCachingPolicy {
    NonCaching,    // Slow Memory RW - Totally safe for MMIO/DMA
    WriteThrough,  // Fast Memory R, Slow Memory/MMIO W - Not safe for MMIO Read
    WriteBack      // Fast Memory-only R/W - Not for MMIO
}

extern "C" {
    fn switch_to_userspace(rip: usize, rsp: usize, pml4_base: usize,
                            tss_rsp0_addr: usize);
}

//
// Symmetric Multiprocessing Support
//
fn start_smp(bsp_cpu_id: usize, max_cpus: usize) {
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
            dbg!("<IRQ#{}.{} -> GSI#{} ON {}{}>\n",
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
            if lapic.lapic_id == bsp_cpu_id as u8 {
                dbg!("BSP [{}] LAPIC ID: {} INITIALIZING\n",
                    lapic.cpu_id, lapic.lapic_id);
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
        let lapic_en;
        {
            let this_machine = THIS_MACHINE.lock();
            lapic_id = (*this_machine).acpi_info.lapic[i].lapic_id;
            lapic_en = (*this_machine).acpi_info.lapic[i].enabled;

        }
        if lapic_id > 0 && lapic_id < max_cpus as u8 && lapic_en == true {
            let current_cpu_cnt = cpu_count();
            dbg!("Senging INIT-SIPI to LAPIC[{}]\n", lapic_id);
            lapic0.send_init_ipi(lapic_id);
            cpu_busywait_us(10_000); // Wait for the cpu to initialize (~10ms)
            lapic0.send_startup_ipi(lapic_id, 0x8); 
            cpu_busywait_us(1_000); // Wait for the AP to initialize
            if cpu_count() == current_cpu_cnt {
                // Send another SIPI
                dbg!("Sending another SIPI to LAPIC[{}]\n", lapic_id);
                lapic0.send_startup_ipi(lapic_id, 0x8);
            }
           
            loop {
                cpu_busywait_us(1_000);
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
    let isr_vector_offset = 34; // See how IDT is set up in boot.S
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
                true, 0
            );
        }
        // TEMP: Register the keyboard interrupt
        ioapic0.register_isr(1, 35, X86IoApic::PRIORITY_FIXED,
               X86IoApic::POLARITY_HIGH , X86IoApic::TRIGGER_EDGE,
               false , 0);
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

fn x86_acpi_parse(rsdp:Option<usize>) -> Option<AcpiInfo>{
    // 1) Find "RSD PTR " in low memory - on a 
    let mut valid_rsdp = false;
    let mut ptr: *mut u64;
    match rsdp {
        Some(addr) => {
            // RSD PTR already provided by the bootloader/EFI firmware
            ptr = addr as *mut u64;
            unsafe {
                if *ptr == 0x2052545020445352 {
                    valid_rsdp = true;
                }
            }
        }
        None => {
            // Have to search for the pointer in the lower memory
            ptr = 400 as *mut u64;
            for _i in 0..0x20000 {
                unsafe {
                    if *ptr == 0x2052545020445352 { 
                        valid_rsdp = true;
                        break;
                    }
                    ptr = ptr.wrapping_add(2); // 16-byte boundary
                }
            }
        }
    }

    if valid_rsdp == false {
        return None;
    }

    let mut ret = AcpiInfo::new();
    let mut sdt: *mut u32;
    let num_tables: u32;
    let addr_size;
    // 2) Find RSDT/XSDT (RSD[16] as u32)
    // FORMAT OF THE (X)ROOT SDT
    // OFF TYPE&NAME
    // 0   char Signature[4]; <-- "RSDT"/"XSDT"
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
        let rev = *((ptr as *mut u8).wrapping_add(15));
        if rev == 0 {
            // REVISION 1 - RSDT
            sdt = *(ptr.wrapping_add(2)) as *mut u32;
            addr_size = 4; // Addrs are 32-bit
        } else if rev == 2 {
            sdt = *((ptr.wrapping_add(3))) as *mut u32;
            addr_size = 8; // Addrs are 64-bit            
        } else {
            dbg!("ACPI REVISION UNKNOWN ({})\n", rev);
            return None;
        }
        num_tables = (*sdt.wrapping_add(1) - 36) / addr_size; 
        dbg!("ACPI REVISION {} - Root Table @ {:p} - RSDT Length: {} ({} tables)\n",
                rev, sdt, *sdt.wrapping_add(1), num_tables
        );
        
        sdt = sdt.wrapping_add(9);
        for _ in 0..num_tables {
            let tbl: *mut u32 = *sdt as *mut u32;
            // System Description Table header signatures to look for:
            // "APIC" = 0x43495041 => MADT (Multi-APIC Description Table)
            // "FACP" = 0x50434146 => FADT (Fixed ACPI Description Table)
            // "HPET" = 0x54455048 => HPET (High Resolution Event Timer)
            match *tbl {
                0x43495041  => x86_acpi_parse_madt(&mut ret, tbl as u32), 
                0x50434146  => x86_acpi_parse_facp(&mut ret, tbl as u32),
                0x54455048  => x86_acpi_parse_hpet(&mut ret, tbl as u32),
                _           => ()
            };
            sdt = sdt.wrapping_add(addr_size as usize / 4);
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
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct TaskContext {
    ep:     fn(),   // Initial RIP value, i.e., Entry-point
    rsp:    usize,  // Last RSP (Stack Pointer) value
    tid:    usize,  // For debugging purposes
}

impl TaskContext {

    pub const fn new() -> Self {
        Self {
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
        self.tid = id;
    }

    pub fn tid(&self) -> usize {
        self.tid
    }
    // This function is called as a wrapper of the task's callback to handle
    // the return of the task (i.e., exit)
    fn launch_task(task: &mut TaskContext) {
        // archlog!("Starting task[{}]: state {}, rip:{:X}, rsp:{:X}\n",
        //     task.tid, task.state, task.ep as usize, task.rsp);
        (task.ep)();
        // Terminate the task
        Task::exit();
        panic!("Continued a dead task's code where it have been unreachable!");
    }
}

fn empty_task() {

}

extern "C" {
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
    unsafe{
        switch_context(from as *const TaskContext as usize,
                        to as *const TaskContext as usize);
    }
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

pub fn syscall(params: Syscall) {
    match params {
        Syscall::Exit { status }                                        => {
            syscall_trigger_int(SyscallOpCode::Exit as usize, status, 0, 0, 0)
        },
        Syscall::Open { path_ptr, mode, ret_ptr }                       => {
            syscall_trigger_int(SyscallOpCode::Open as usize,
                                path_ptr, mode, ret_ptr, 0)
        },
        Syscall::Read { fd, buf_ptr, buf_len, ret_ptr }                 => {
            syscall_trigger_int(SyscallOpCode::Read as usize,
                                fd, buf_ptr, buf_len, ret_ptr)
        },
        Syscall::Write { fd, buf_ptr, buf_len, ret_ptr }                => {
            syscall_trigger_int(SyscallOpCode::Write as usize,
                                fd, buf_ptr, buf_len, ret_ptr);
        },
        Syscall::Exec { fd, cmd_buf_ptr, buf_len, ret_ptr }               => {
            syscall_trigger_int(SyscallOpCode::Exec as usize,
                                fd, cmd_buf_ptr, buf_len, ret_ptr);
        },
        Syscall::Close { fd }                                           => {
            syscall_trigger_int(SyscallOpCode::Close as usize , fd, 0, 0, 0);
        }
    }
}

fn syscall_trigger_int(opcode: usize,
                        arg0: usize, arg1: usize, arg2: usize, arg3: usize) {
    unsafe{
        asm!(
            "int 0x20", // See boot.S
            in("rax") opcode,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            in("rcx") arg3,
        );
    };
}

#[unsafe(no_mangle)]
extern "C"
fn ksyscall_handler(arg0: usize, arg1: usize, arg2: usize, arg3: usize) {
    let opcode: usize;
    unsafe {
        asm!(
            "mov {0}, rax",
            out(reg)opcode
        );
    }
    if opcode < SyscallOpCode::Max as usize {
        unsafe {
            X64_SYSCALL_HANDLER[opcode](arg0, arg1, arg2, arg3);
        }
    }
}


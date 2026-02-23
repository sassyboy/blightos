// 
// BlightOS Kernel
// 
// Support module for the x86_64 architecture
//
#![allow(dead_code)]

use core::arch::asm;
use core::time::Duration;
use crate::arch::*;
use crate::mem::phys::*;
use crate::sched::Task;
use crate::{SyscallHandlerFn, SyscallOpCode, util::*};
use crate::{dump_memory, kstart};
use crate::drivers::video::framebuffer::*;
use core::fmt::Write;

mod systimer;
mod mmu;

// Re-export the following modules under crate::arch::
pub use self::systimer::*;
pub use self::mmu::*;

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
    hpet:       X86HPET,
}

impl MachineContext {
    pub const fn new() -> Self {
        Self {
            cpu_count:  1, // There's at least one cpu (BSP), lol!
            acpi_info:  AcpiInfo::new(),
            ioapic:     X86IoApic::new(),
            hpet:       X86HPET::new()
        }
    }
}

pub static THIS_MACHINE: Spinlock<MachineContext> = 
                                        Spinlock::new(MachineContext::new());

percpu_global! {
    THIS_PERCPU_BASE: usize = 0; // To avoid rdmsr(IA32_GS_BASE) every time
    pub THIS_LAPIC:  X86LocalApic = X86LocalApic::new();
    pub THIS_IOAPIC: X86IoApic    = X86IoApic::new();
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

    // Initialize the per-cpu sections
    let bsp_cpu_id = cpu_lapic_id(); // LAPIC ID
    percpu_init_sections();
    percpu_init_cpu(bsp_cpu_id);
    THIS_CPU_ID.write(bsp_cpu_id);

    // Physical Memory Map Buffer
    let mut mem_map: [PMMapElement; 32] = [
        PMMapElement {base: 0, len: 0, avail: false}; 32];
    let mut mem_map_count = 0;
    let mut acpi_rsdp: Option<usize> = None;

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
                unsafe {
                    init_framebuffer_from_mb2(&((*tag).tdata.frame_buffer));
                }
            },
            Mulitboot2TagType::VBE              => {
                // Nothing to do
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
                // Not needed anymore
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
    let (rows, cols) = FrameBuffer::screen_size();
    kearly_console::init();
    klog!("VESA Graphics: {} x {}\n", cols, rows);

    // Todo - fetch kernel's boot command-line/parameters
    // Todo - Pass a list kernel modules (e.g., ramdisk) Grub loaded for us
    
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

    // TSC frequency calculation may require HPET to be functional, which
    // in turn depends on ACPI enumeration.
    THIS_TSC.borrow_mut().init();

    // Fix the syscall interrupt entry (0x20) in IDT to accept calls from Ring3
    extern "C" {
        static idt_base : usize;
    }
    unsafe {
        let idte: *mut u64 = &idt_base as *const usize as *mut u64;
        *(idte.wrapping_add(0x20 * 2)) |= 0x600000000000; // DPL = 3
    }

    // Start the kernel.
    kstart(bsp_cpu_id, Some(&mem_map[0..mem_map_count]));

    // kstart shouldn't really return but if does, we should panic
    panic!(); 
}

#[unsafe(no_mangle)]
extern "C" fn rust_x864_entry_ap(_arg: usize) {
    let cpuid = cpu_lapic_id();
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
    kstart(cpuid as usize, None);
}

pub fn init_framebuffer_from_mb2(inp: &Multiboot2FrameBuffer) {
	let mut fb = FrameBuffer::new();
    fb.base_address         = inp.addr as usize;
    fb.pitch                = inp.pitch;
    fb.width                = inp.width;
    fb.height               = inp.height;
    fb.bpp                  = inp.bpp;
    fb.red_field_position   = inp.color_info[0];
    fb.red_mask_size        = inp.color_info[1];
    fb.green_field_position = inp.color_info[2];
    fb.green_mask_size      = inp.color_info[3];
    fb.blue_field_position  = inp.color_info[4];
    fb.blue_mask_size       = inp.color_info[5];
    fb.background_rgb       = (240, 240, 240);
    fb.foreground_rgb       = (0, 0, 255);
    // Map the buffer memory
    // Register the framebuffer
    FrameBuffer::register(&fb);
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
    SystemTimer::exec_handler();
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
        cpu = cpu_lapic_id();
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
        mask1 = x86_ioport_read::<u8>(PIC1_PORT_DAT);
        mask2 = x86_ioport_read::<u8>(PIC2_PORT_DAT);

        // Send the initialization command and data sequence 
        x86_ioport_write::<u8>(PIC1_PORT_CMD, PIC_CMD_INIT);
        x86_ioport_write::<u8>(PIC2_PORT_CMD, PIC_CMD_INIT);
        // Set the vector offests
        x86_ioport_write::<u8>(PIC1_PORT_DAT, idt_vector_offset);
        x86_ioport_write::<u8>(PIC2_PORT_DAT, idt_vector_offset + 8);
        // IRQ2 on the master PIC is connected to the slave (PIC2)
        x86_ioport_write::<u8>(PIC1_PORT_DAT, 4);
        // Cascade ID of the slave
        x86_ioport_write::<u8>(PIC2_PORT_DAT, 2);
        // Set the mode of both PICs to 8086
        x86_ioport_write::<u8>(PIC1_PORT_DAT, 1);
        x86_ioport_write::<u8>(PIC2_PORT_DAT, 1);

        // Restore the IRQ masks
        x86_ioport_write::<u8>(PIC1_PORT_DAT, mask1);
        x86_ioport_write::<u8>(PIC2_PORT_DAT, mask2);
    }

    pub fn send_eoi(irq: u8) {
        if irq >= 8 {
            x86_ioport_write::<u8>(PIC2_PORT_CMD, PIC_CMD_EOI);
        }
        x86_ioport_write::<u8>(PIC1_PORT_CMD, PIC_CMD_EOI);
    }

    pub fn mask_irq(irq: u8) {
        if irq < 8 {
            x86_ioport_write::<u8>(PIC1_PORT_DAT, 
                x86_ioport_read::<u8>(PIC1_PORT_DAT) | (1 << irq));
        } else {
            x86_ioport_write::<u8>(PIC2_PORT_DAT, 
                x86_ioport_read::<u8>(PIC2_PORT_DAT) | (1 << (irq - 8)));
        }
    }

    pub fn mask_all() {
        x86_ioport_write::<u8>(PIC1_PORT_DAT, 0xFF);
        x86_ioport_write::<u8>(PIC2_PORT_DAT, 0xFF);
    }

    pub fn unmask_irq(irq: u8) {
        if irq < 8 {
            x86_ioport_write::<u8>(PIC1_PORT_DAT, 
                x86_ioport_read::<u8>(PIC1_PORT_DAT) & !(1 << irq));
        } else {
            x86_ioport_write::<u8>(PIC2_PORT_DAT, 
                x86_ioport_read::<u8>(PIC2_PORT_DAT) & !(1 << (irq - 8)));
        }
    }
}



#[derive(Debug)]
struct X86HPET {
    valid:          bool,
    comp_count:     u8,
    comp_size:      u8,
    hpet_num:       u8,
    min_prd_tick:   u16,
    irq:            u8,
    mmio_base:      usize,
    period_ns:        u32,
}

impl X86HPET {
    pub const fn new() -> Self {
        Self {
            valid:          false,
            comp_count:     0,
            comp_size:      0,
            hpet_num:       0,
            min_prd_tick:   0,
            irq:            0,
            mmio_base:      0,
            period_ns:      0
        }
    }

    const REG_GEN_CAP:          usize = 0x000;
    const REG_GEN_CONFIG:       usize = 0x010;
    const REG_MAIN_COUNTER_VAL: usize = 0x0F0;


    const REG_TIM0_CONFIG:      usize = 0x100;
    const REG_TIM0_COMP_VAL:    usize = 0x108;
    const REG_TIM0_FSB_INT_ROUT:usize = 0x110;

    pub fn init(&mut self, acpi_hpet_base: usize) {
        // Relevant HPET fields
        // Offset (size)
        // 36     (4)    :Event Timer Block ID
        //                [31:16] PCI Vendor ID of 1st Timer Block
        //                [15]    Legacy Replacement IRQ routing Capable
        //                [13]    COUNT_SIZE_CAP (0: 32-bit, 1: 64-bit)
        //                [12:8]  # of comparators in 1st Timer Block
        //                [7:0]   Hardware Rev ID
        //
        // 40     (12)    MMIO Base Address in the form of ACPIGenericAddress,
        //                1KB region, regardless of the # of comparators
        // 52     (1)     HPET sequence number: 0 = 1st, 1 = 2nd, etc.
        // 53     (2)     Main Counter Minimum Clock Tick in periodic mode

        // Fetch mmio_base
        let gen_addr = AcpiGenericAddress::from_acpi_entry(acpi_hpet_base + 40);
        match gen_addr {
            AcpiGenericAddress::Memory { addr }     => {
                self.mmio_base = MMUMapping::dma_from_kernel_phys(addr) ;
            },
            _ => {
                self.valid = false;
                return;
            }
        }

        // Fetch comp_count and comp_size
        let blk_id: u32;
        let u32ptr = (acpi_hpet_base + 36) as *const u32;
        unsafe {
            blk_id = u32ptr.read_volatile();
        }
        self.comp_count = ((blk_id >> 8) & 0x1F) as u8 + 1;
        if blk_id & 0x2000 > 0 {
            self.comp_size = 64;
        } else {
            self.comp_size = 32;
        }
        // TODO look at bld_id & 0x8000 if IRQ support is needed

        // Fetch min_prd_tick
        let u16ptr = (acpi_hpet_base + 53) as *const u16;
        unsafe {
            self.min_prd_tick = u16ptr.read_volatile();
        }
        // Fetch hpet_num
        let u8ptr = (acpi_hpet_base + 52) as *const u8;
        unsafe {
            self.hpet_num = u8ptr.read_volatile();
        }

        // TODO - Make sure none of the timers generate interrupts
        self.disable_counting();
        self.reset_current_count();
        self.valid = true;
        self.period_ns = round_up!(
                        (self.read_reg(Self::REG_GEN_CAP) >> 32) as u32,
                        1_000_000) / 1_000_000;
    }

    fn read_reg(&self, reg_no: usize) -> u64 {
        unsafe {
            return((self.mmio_base + reg_no) as *const u64).read_volatile()
        }
    }
    fn write_reg(&self, reg_no: usize, val: u64) {
        unsafe {
            ((self.mmio_base + reg_no) as *mut u64).write_volatile(val);
        }
    }

    fn current_count(&self) -> u64 {
        self.read_reg(Self::REG_MAIN_COUNTER_VAL)
    }

    fn reset_current_count(&self) {
        self.write_reg(Self::REG_MAIN_COUNTER_VAL, 0);
    }

    fn enable_counting(&self) {
        let gen_config_val = self.read_reg(Self::REG_GEN_CONFIG);
        self.write_reg(Self::REG_GEN_CONFIG, gen_config_val | 1);
    }

    fn disable_counting(&self) {
        let gen_config_val = self.read_reg(Self::REG_GEN_CONFIG);
        self.write_reg(Self::REG_GEN_CONFIG, gen_config_val & !(1 as u64));
    }

    fn duration_to_ticks(&self, d: Duration) -> u64 {
        (d.as_nanos() / self.period_ns as u128) as u64
    }
    

}


pub struct X86IoApic{
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

    fn init(&mut self, ioapic: &AcpiIOApic) {
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
            let high:   u32;
            let mut low:u32;

            low = (isr_vector as u32) | priority | pin_polarity | pin_trigger;
            if masked {low |= 0x10000;}  
            high = (dest_cpu_acpi_id_mask as u32) << 24;

            self.write_reg((entry_index * 2 + 0x10) as u8, low);
            self.write_reg((entry_index * 2 + 0x11) as u8, high);
        }

    }

    pub fn set_irq_mask(&mut self, gsi: u32, masked: bool) {
        let entry_index = gsi - self.gsi_base;
        if gsi < self.max_irqs as u32 {
            let mut low = self.read_reg((entry_index * 2 + 0x10) as u8);
            low = match masked {
                true  => low | 0x10000,
                false => low & 0x0FFFF
            };
            self.write_reg((entry_index * 2 + 0x10) as u8, low);
        }
    }
}

pub struct X86LocalApic {
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

    fn init(&mut self, lapic: &AcpiLocalApic, lapic_mmio: u32) {
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
		cpu_busywait(Duration::from_micros(200));
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
    THIS_CPU_ID: usize = 0; // To avoid issuing cpuid every time in cpu_id()
    pub THIS_CPU_IDLE_TSC_TICKS:    usize = 0;
}


pub fn machine_reboot() {
    let reboot_port;
    let reboot_val;    
    {
        let this_machine = THIS_MACHINE.lock();
        reboot_port = (*this_machine).acpi_info.reboot_reg;
        reboot_val  = (*this_machine).acpi_info.reboot_val;
    }
    
    match reboot_port {
        AcpiGenericAddress::IOPort { port_num }     => {
            x86_ioport_write(port_num, reboot_val);    
        },
        AcpiGenericAddress::Memory { addr }         => {
            if addr < (1 << 32) {
                unsafe {
                    (addr as *mut u64).write_volatile(reboot_val as u64);
                }
            }
        },
        _   => {
            klog!("Reboot method not supported\n");
        }
    }
    // klog!("FADT_sig:{:X}, reboot_port = {},{},{},{},{:X}, reboot_val = {:X}\n",
    //         fadt_sig, addr_space, bit_width, bit_off, access_sz, reboot_port,
    //         reboot_val);
    // //
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
pub fn cpu_restore_ints(enabled: bool) {
    if enabled {
        cpu_enable_ints();
    } else {
        cpu_disable_ints();
    }
}
pub fn cpu_ints_enabled() -> bool {
    let rflg = x64_read_rflags();
    rflg & 0x200 > 0
}
pub fn cpu_unmask_irq(irq: u32) {
    THIS_IOAPIC.borrow_mut().set_irq_mask(irq, false);
}
pub fn cpu_mask_irq(irq: u32) {
    THIS_IOAPIC.borrow_mut().set_irq_mask(irq, true);
}

pub fn cpu_halt() {
    unsafe {
        asm!("hlt");
    }
}

pub fn cpu_stack_pointer() -> usize{
    let sp: usize;
    unsafe {
        asm!(
            "mov {sp}, rsp",
            sp = out(reg)sp
        );
    }
    sp
}

pub fn cpu_count() -> usize {
    (*(THIS_MACHINE.lock())).cpu_count
}

// Returns LAPIC CPU ID of the current CPU calling the routine using the
// CPUID instruction with Extended Topology Leaf (0BH)
pub fn cpu_lapic_id() -> usize {
    let apic_cpu_id: u32;
    (_, _, _, apic_cpu_id) = x86_cpuid_inst(0xB, 0x0);
    apic_cpu_id as usize
}

pub fn cpu_id() -> usize {
    *THIS_CPU_ID.borrow_mut()
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
pub fn cpu_busywait(delay: Duration) {
    let target_tsc = cpu_read_timestamp() + 
                        SystemTimer::duration_to_timestamp_ticks(delay);
    while cpu_read_timestamp() < target_tsc {
        core::hint::spin_loop();
    }
}

pub trait X86IOPortAccess {
    fn x86_ioport_read(port: u16) -> Self;
    fn x86_ioport_write(port: u16, val: Self);
}

impl X86IOPortAccess for u8 {
    fn x86_ioport_read(port: u16) -> Self {
        let data: u8;
        unsafe {
            asm!("in al, dx", out("al") data, in("dx") port);
        }
        data
    }
    fn x86_ioport_write(port: u16, data: Self) {
        unsafe {
            asm!("out dx, al", in("dx") port, in("al") data);
        }
    }
}

impl X86IOPortAccess for u32 {
    fn x86_ioport_read(port: u16) -> Self {
        let data: u32;
        unsafe {
            asm!("in eax, dx", out("eax") data, in("dx") port);
        }
        data
    }
    fn x86_ioport_write(port: u16, data: Self) {
        unsafe {
            asm!("out dx, eax", in("dx") port, in("eax") data);
        }
    }
}

pub fn x86_ioport_read<T: X86IOPortAccess>(port: u16) -> T {
    T::x86_ioport_read(port)
}
pub fn x86_ioport_write<T: X86IOPortAccess>(port: u16, data: T) {
    T::x86_ioport_write(port, data);
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
        // Disable all interrupts
        x86_ioport_write::<u8>(self.port + 1, 0x00);
        // Enable DLAB (set baud rate divisor)
        x86_ioport_write::<u8>(self.port + 3, 0x80);
        // Set divisor to 3 (lo byte) 38400 baud
        x86_ioport_write::<u8>(self.port + 0, 0x03); 
        // Set divisor to 3 (hi byte) 38400 baud
        x86_ioport_write::<u8>(self.port + 1, 0x00);
        // 8 bits, no parity, one stop bit
        x86_ioport_write::<u8>(self.port + 3, 0x03);
        // Enable FIFO, clear them, with 14-byte threshold
        x86_ioport_write::<u8>(self.port + 2, 0xC7);
        // IRQs enabled, RTS/DSR set
        x86_ioport_write::<u8>(self.port + 4, 0x0B);
        // Set in loopback mode, test the serial chip
        x86_ioport_write::<u8>(self.port + 4, 0x1E);
        // Test serial chip (send byte 0xAE and check if serial returns same byte)
        x86_ioport_write::<u8>(self.port + 0, 0xAE);
        // If serial is not faulty set it in normal operation mode
        // (not-loopback with IRQs enabled and OUT#1 and OUT#2 bits enabled)
        x86_ioport_write::<u8>(self.port + 4, 0x0F);


    }

    pub fn putc(&self, c: u8) {
        while x86_ioport_read::<u8>(self.port + 5) & 0x20 == 0 {
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
    use crate::drivers::video::framebuffer::*;
    use crate::util::Spinlock;

    static CURSOR : Spinlock<(u32, u32)> = Spinlock::new((0,0));
    // CURSOR.0 -> row, .1 -> column

    pub fn init() {
        FrameBuffer::clean_screen();
        let mut cursor = CURSOR.lock();
        cursor.0 = 0;
        cursor.1 = 0;
        
    }


    pub fn print_str(msg: &[u8]) {
        let (sh, sw) : (u32, u32) = FrameBuffer::screen_size();
        let (fh, fw) : (u32, u32) = FrameBuffer::font_size();
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


//
// IRQ Interface
//
type IsrHandlerFn = fn(u16);

fn isr_default_imp(_: u16) { }
static mut X86_ISR_HANDLER: [IsrHandlerFn; 24] = [isr_default_imp; 24];

pub fn irq_controller_init() {
    x86_pic::init(32);
}

pub fn isr_register(irq: u16, handler_fn: IsrHandlerFn) {
    if irq < 32 { /* Last IRQ handler in boot.S is irq_handler31 */
        unsafe {
            X86_ISR_HANDLER[irq as usize] = handler_fn;
        }
    }
}

pub fn irq_reroute(gsi: u32, vector: u8, edge_triggered: bool) {
    let ioapic0 = THIS_IOAPIC.borrow_mut();
    // really bad code - fix the types and prototye
    let trig;
    if edge_triggered {
        trig = X86IoApic::TRIGGER_EDGE;
    } else {
        trig = X86IoApic::TRIGGER_LEVEL;
    }
    ioapic0.register_isr(gsi, vector + 34,
                X86IoApic::PRIORITY_FIXED,
                X86IoApic::POLARITY_HIGH , trig, false , 0);
}

pub fn cpu_trigger_systimer_irq() {
    unsafe{
        asm!(
            "int 0x21" // See boot.S
        );
    };
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
        (0x8000 as *mut u8).copy_from_nonoverlapping(
                                src_start as *const u8, src_end - src_start);
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
            cpu_busywait(Duration::from_millis(10)); // Wait for the cpu to initialize (~10ms)
            lapic0.send_startup_ipi(lapic_id, 0x8); 
            cpu_busywait(Duration::from_millis(1)); // Wait for the AP to initialize
            if cpu_count() == current_cpu_cnt {
                // Send another SIPI
                dbg!("Sending another SIPI to LAPIC[{}]\n", lapic_id);
                lapic0.send_startup_ipi(lapic_id, 0x8);
            }
           
            loop {
                cpu_busywait(Duration::from_millis(1));
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
    }
    //// TODO Route NMIs

}

//
// System Configuration (ACPI)
//
pub const MAX_CPU_COUNT: usize = 8;
pub const MAX_IRQ_COUNT: usize = 16;

#[derive(Clone, Copy)]
enum AcpiGenericAddress{
    Memory{addr: usize},
    IOPort{port_num: u16},
    Unsupported,
}
impl AcpiGenericAddress {
    pub fn from_acpi_entry(entry_mem_addr: usize) -> Self {
        // Format:
        //   uint8_t AddressSpace; // 0:Memory, 1:System I/O, 2:PCI BUS 0
        //   uint8_t BitWidth;     // Must be 8
        //   uint8_t BitOffset;    // Must be 0
        //   uint8_t AccessSize;
        //   uint64_t Address; <-- Port number/MMIO_BASE
        let addr_space: u8;
        // let bit_width:  u8;
        // let bit_off:    u8;
        // let access_sz:  u8;
        let addr: u64;
        unsafe {
            addr_space = ((entry_mem_addr + 0) as *mut u8).read_volatile();
            // bit_width  = ((addr + 116 + 1) as *mut u8).read_volatile();
            // bit_off    = ((addr + 116 + 2) as *mut u8).read_volatile();
            // access_sz  = ((addr + 116 + 3) as *mut u8).read_volatile();
            addr = ((entry_mem_addr + 4) as *mut u64).read_volatile();
        }
        if addr_space == 0 {
            return AcpiGenericAddress::Memory { addr: addr as usize };
        } else if addr_space == 1 {
            return AcpiGenericAddress::IOPort { port_num: addr as u16 };
        } else {
           return AcpiGenericAddress::Unsupported;
        }
    }
}

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
    reboot_reg: AcpiGenericAddress,
    reboot_val: u8,
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
            nmi_map_cnt: 0,
            reboot_reg: AcpiGenericAddress::Unsupported,
            reboot_val: 0
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

    // RESET REGISTER is located at 116 in the FADT with the following format:
    //   uint8_t AddressSpace; // 0:Memory, 1:System I/O, 2:PCI BUS 0
    //   uint8_t BitWidth;     // Must be 8
    //   uint8_t BitOffset;    // Must be 0
    //   uint8_t AccessSize;
    //   uint64_t Address; <-- Port number
    // 
    // RESET VALUE is located at 128
    let addr_space: u8;
    // let bit_width:  u8;
    // let bit_off:    u8;
    // let access_sz:  u8;
    let reboot_port:usize;

    unsafe {
        addr_space = ((addr + 116 + 0) as *mut u8).read_volatile();
        // bit_width  = ((addr + 116 + 1) as *mut u8).read_volatile();
        // bit_off    = ((addr + 116 + 2) as *mut u8).read_volatile();
        // access_sz  = ((addr + 116 + 3) as *mut u8).read_volatile();
        reboot_port= ((addr + 116 + 4) as *mut usize).read_volatile();
        acpi.reboot_val = ((addr + 128) as *mut u8).read_volatile();
    }
    if addr_space == 0 {
        acpi.reboot_reg = AcpiGenericAddress::Memory { addr: reboot_port };
    } else if addr_space == 1 {
        acpi.reboot_reg = AcpiGenericAddress::IOPort { 
                                                port_num: reboot_port as u16 };
    } else {
        acpi.reboot_reg = AcpiGenericAddress::Unsupported;
    }
}

fn x86_acpi_parse_hpet(acpi: &mut AcpiInfo, addr: u32) {
    acpi.hpet_base = addr;
    let mut m = THIS_MACHINE.lock();
    m.hpet.init(addr as usize);
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


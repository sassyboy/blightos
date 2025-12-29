// Rust stub for the x86_64 architecture

use core::mem::size_of;
use core::arch::asm;
use crate::pmm::PMMapElement;
use crate::sched;
use crate::{dump_memory, kstart};

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
extern "C" fn kexcep_invalid_opcode(rsp: usize) {
    dump_memory(rsp, 8);
    panic!("kexcep_invalid_opcode");
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
extern "C" fn kexcep_page_fault() {
    panic!("kexcep_page_fault");
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
// Temporary debugging macros
//
// use core::fmt::Write;
// struct ArchDebugConsole;
// impl Write for ArchDebugConsole {
//     fn write_str(&mut self, _s: &str) -> core::fmt::Result {
//         kearly_console::print_str(_s.as_bytes());
//         Ok(())
//     }
// }
// macro_rules! archlog {
//     ($($arg:tt)*) => {
//         let mut kern_console = ArchDebugConsole{};
//         let _ = write!(&mut kern_console, $($arg)*);
//     };
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
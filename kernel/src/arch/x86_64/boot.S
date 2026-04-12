/**
 * boot.S
 *
 * Description:
 * Entry-Point from a multiboot bootloader which switches the BSP processor to
 * Long mode and calls x64init() in kstub.c
 *
 */

.code32
.section .multiboot_header

.globl boot_start
boot_start:
  jmp bsp_start32

/* Multiboot 2.0 Header - Section 3.1 - OS Image Format
 * - Must appear in the first 32KB of the kernel image.
 * - The header and each tag must be 8-byte aligned
 */
.align 8
_mb2h_start:
  .long 0xE85250D6                                /* magic */
  .long 0x00000000                                /* Arch - 0: i386 */
  .long _mb2h_end - _mb2h_start                   /* Header Length (bytes) */
  .long -(0xE85250D6 + (_mb2h_end - _mb2h_start)) /* Checksum */
  
/* Framebuffer Tag (type 5, len 20) */
_mb2h_framebuffer:
  .short 5  
  .short 1  /* Optional - Bootloader is free to choose any mode */
  .long  _mb2h_frambuffer_end - _mb2h_framebuffer
#ifdef STDOUT_VGA_RGB
  .long  VGA_RGB_WIDTH
  .long  VGA_RGB_HEIGHT
  .long  32 /* color depth */
#else
  .long 80
  .long 25
  .long 0
#endif
_mb2h_frambuffer_end:
_mb2h_null:
.align 8
  .short  0
  .short  0
  .long   8
_mb2h_end:

.text
bsp_start32:
  cli
  /* Setup kernel initialization stack */
  movl $_KINIT_STACK_START+PER_CPU_KERNEL_STACK_SIZE, %esp
  /* Initialize 64-BIT IDT */
  call setup_idt
  lidt idt_pointer
  /* Build initial 4-Level Paging tables */
  call setup_initial_4lp
  /* Now enter Long Mode */
  /* 1) Enable PAE */
  mov %cr4, %edx
  or  $0x20,%edx
  mov %edx, %cr4
  /* 2) Enable Long Mode (Set IA32_EFER.LME) */
  mov $0xC0000080, %ecx
  rdmsr
  or $0x00000100, %eax
  wrmsr
  /* 3) Enable Protection and Paging(This will FINALLY activate the LONG MODE) */
  mov %cr0, %eax
  or  $0x80000001, %eax
  mov %eax, %cr0
  /* 4) Load GDT and jump to 64-bit world */
  lgdt gdt64_pointer
  ljmp $0x8,$1f

.code64
1:
  mov   $0x10, %ax
  mov   %ax, %ds
  mov   %ax, %es
  mov   %ax, %fs
  mov   %ax, %gs
  mov   %ax, %ss
  lidt  idt_pointer

  xor   %rdi, %rdi
  mov   %ebx, %edi              /* The multiboot structure address */
  mov   $MAX_CPU_COUNT, %rsi
  callq  rust_x864_entry_bsp
  hlt

/**
 * setup_initial_4lp
 *
 * Initializes the neceassary 4-level page tables for direct mapping of the
 * first 4GBs of the physical memory into the kernel's initial virtual memory.
 * Large/Huge Pages are enabled on x64 by default! So, no x86 PSE nonsense!
 */
.code32
setup_initial_4lp:
	pushl	%ebp
	pushl %eax
  pushl %ecx

  xorl  %ecx, %ecx
	jmp	.cleanup_loop_cond
.cleanup_loop_body:
/* Zero-out all the entries in pdpt[0], pml4 */
	movl	$0, kpdpt0_tbl(,%ecx,4)
	movl	$0, kpml4_tbl(,%ecx,4)
	incl  %ecx
.cleanup_loop_cond:
	cmpl	$1024, %ecx
	jle	.cleanup_loop_body

/* Level 4) Set pml4e[0] point to page directory pointer table (first 512 GB) */
  movl	$kpdpt0_tbl, %eax
  orl   $0x3, %eax                              /* Present|Writable|WriteBack */
  movl	%eax, kpml4_tbl

/* Level 3) PDPT[0] maps the first 4GBs of virtual address space for kernel   */
                                      /* 0x83 = Present|Writable|PS|WriteBack */
  movl  $0x00000083, %eax
  movl  %eax, kpdpt0_tbl+0*8
  movl  $0x40000083, %eax
  movl  %eax, kpdpt0_tbl+1*8
  movl  $0x80000083, %eax
  movl  %eax, kpdpt0_tbl+2*8
  movl  $0xC0000083, %eax
  movl  %eax, kpdpt0_tbl+3*8

/* Load base address of PML4 table into cr3 */
	mov $kpml4_tbl, %eax
	mov %eax, %cr3

/* Clean-up and return */
	nop
	popl  %ecx
  popl  %eax
	popl	%ebp
	ret

/**
 * setup_idt
 *
 * Installs a 66-item IDT table in which all items are Interrupt gates and
 * point to various interrupt entry point according to the table idt_vectors
 */
.code32
setup_idt:
  pushal
  lea idt_base,%edi
  mov $66,%ecx
  lea idt_vectors, %ebx
1:
  movl (%ebx),%edx         /* vector offset[31:0] */
  movw %dx, (%edi)         /* write vector offset[15:0] */
  movw $0x8, 2(%edi)       /* selector = 0x0008 = cs */
  movw $0x8E00, 4(%edi)    /* interrupt gate - dpl=0, present */
  shrl $16, %edx
  movw %dx, 6(%edi)        /*  write vector offset[31:16] */
  addl $16, %edi          /* Increment IDT Entry Pointer */
  addl $8, %ebx           /* Increment idt_vector Pointer */
  dec %ecx
  jne 1b
  popal
  ret

.code64

.macro SAVE_GP_REGS
  pushq   %rax
  pushq   %rbx
  pushq   %rcx
  pushq   %rdx
  pushq   %rsi
  pushq   %rdi
  pushq   %rbp
  pushq   %r8
  pushq   %r9
  pushq   %r10
  pushq   %r11
  pushq   %r12
  pushq   %r13
  pushq   %r14
  pushq   %r15
.endm

.macro RSTR_GP_REGS
  popq    %r15
  popq    %r14
  popq    %r13
  popq    %r12
  popq    %r11
  popq    %r10
  popq    %r9
  popq    %r8
  popq    %rbp
  popq    %rdi
  popq    %rsi
  popq    %rdx
  popq    %rcx
  popq    %rbx
  popq    %rax
.endm

/* Saves the registers used in the kernel routines, and places %rdi at the start
 * of the exception frame:
 * Saved R15   --|
 * ...           |
 * Saved  RCX    |-- Pushed by SAVE_GP_REGS
 * Saved  RBX    |
 * Saved  RAX  --|
 * Error Code <----- rdi
 * Retrun RIP  --|
 * Return CS     |
 * Return RFLAGS |-- Pushed by the CPU before entering Ring-0
 * Return RSP    |
 * Return SS   --|
 */
.macro ENTER_EXCEP
  SAVE_GP_REGS
  movq %rsp, %rdi
  addq $(15*8), %rdi /* SAVE_GP_REGS pushed 15 registers on the stack */
.endm

/* Returns from an exception (no error code) handler */
.macro EXIT_EXCEP
  RSTR_GP_REGS
  iretq
.endm

/* Returns from an exception (with error code) handler */
.macro EXIT_EXCEP_WITH_ERR
  RSTR_GP_REGS
  addq    $8, %rsp /* skip over the error code */
  iretq
.endm

.macro CALL_KIRQ_HANDLER irq_num
  SAVE_GP_REGS
  movq    $\irq_num, %rdi
  call    kirq_handler
  RSTR_GP_REGS
.endm

int_default:
  SAVE_GP_REGS
  call kdefault_handler
  RSTR_GP_REGS
  iretq

excep_div_by_zero:
  ENTER_EXCEP
  call kexcep_div_by_zero
  EXIT_EXCEP

excep_nmi:
  ENTER_EXCEP
  call kexcep_nmi
  EXIT_EXCEP

excep_overflow:
  ENTER_EXCEP
  call kexcep_overflow
  EXIT_EXCEP

excep_invalid_opcode:
  ENTER_EXCEP
  call kexcep_invalid_opcode
  EXIT_EXCEP

excep_invalid_tss:
  ENTER_EXCEP
  call kexcep_invalid_tss
  EXIT_EXCEP_WITH_ERR

excep_seg_not_present:
  ENTER_EXCEP
  call kexcep_seg_not_present
  EXIT_EXCEP_WITH_ERR

excep_stack_fault:
  ENTER_EXCEP
  call kexcep_stack_fault
  EXIT_EXCEP_WITH_ERR

excep_gp_fault:
  ENTER_EXCEP
  call kexcep_gp_fault
  EXIT_EXCEP_WITH_ERR

excep_page_fault:
  ENTER_EXCEP
  call kexcep_page_fault
  EXIT_EXCEP_WITH_ERR

excep_x87fpu:
  ENTER_EXCEP
  call kexcep_x87fpu
  EXIT_EXCEP

excep_alignment_chk:
  ENTER_EXCEP
  call kexcep_alignment_chk
  EXIT_EXCEP_WITH_ERR

excep_machine_chk:
  ENTER_EXCEP
  call kexcep_machine_chk
  EXIT_EXCEP_WITH_ERR

excep_simd_fp:
  /* Pass the value of MXCSR value as the argument to the handler */
  SAVE_GP_REGS
  subq    $4, %rsp
  stmxcsr (%rsp)
  movl    (%rsp), %edi
  addq    $4, %rsp
  call kexcep_simd_fp
  RSTR_GP_REGS
  iretq

sys_handler:
  SAVE_GP_REGS
  call    ksyscall_handler
  RSTR_GP_REGS
	iretq

irq_lvt_timer:
  SAVE_GP_REGS
  call    kirq_lvt_timer
  RSTR_GP_REGS
  iretq

irq_handler0:
  CALL_KIRQ_HANDLER 0
	iretq

irq_handler1:
  CALL_KIRQ_HANDLER 1
  iretq

irq_handler2:
  CALL_KIRQ_HANDLER 2
  iretq

irq_handler3:
  CALL_KIRQ_HANDLER 3
  iretq

irq_handler4:
  CALL_KIRQ_HANDLER 4
  iretq

irq_handler5:
  CALL_KIRQ_HANDLER 5
  iretq

irq_handler6:
  CALL_KIRQ_HANDLER 6
  iretq

irq_handler7:
  CALL_KIRQ_HANDLER 7
  iretq

irq_handler8:
  CALL_KIRQ_HANDLER 8
  iretq

irq_handler9:
  CALL_KIRQ_HANDLER 9
  iretq

irq_handler10:
  CALL_KIRQ_HANDLER 10
  iretq

irq_handler11:
  CALL_KIRQ_HANDLER 11
  iretq

irq_handler12:
  CALL_KIRQ_HANDLER 12
  iretq

irq_handler13:
  CALL_KIRQ_HANDLER 13
  iretq

irq_handler14:
  CALL_KIRQ_HANDLER 14
  iretq

irq_handler15:
  CALL_KIRQ_HANDLER 15
  iretq

irq_handler16:
  CALL_KIRQ_HANDLER 16
  iretq

irq_handler17:
  CALL_KIRQ_HANDLER 17
  iretq

irq_handler18:
  CALL_KIRQ_HANDLER 18
  iretq

irq_handler19:
  CALL_KIRQ_HANDLER 19
  iretq

irq_handler20:
  CALL_KIRQ_HANDLER 20
  iretq

irq_handler21:
  CALL_KIRQ_HANDLER 21
  iretq

irq_handler22:
  CALL_KIRQ_HANDLER 22
  iretq

irq_handler23:
  CALL_KIRQ_HANDLER 23
  iretq
irq_handler24:
  CALL_KIRQ_HANDLER 24
  iretq

irq_handler25:
  CALL_KIRQ_HANDLER 25
  iretq

irq_handler26:
  CALL_KIRQ_HANDLER 26
  iretq

irq_handler27:
  CALL_KIRQ_HANDLER 27
  iretq

irq_handler28:
  CALL_KIRQ_HANDLER 28
  iretq

irq_handler29:
  CALL_KIRQ_HANDLER 29
  iretq

irq_handler30:
  CALL_KIRQ_HANDLER 30
  iretq

irq_handler31:
  CALL_KIRQ_HANDLER 31
  iretq

/*******************************************************************************
 * Global Descriptor Table
 * See Section 3.4.5 - Segment Descriptors
 ******************************************************************************/
.data
.align 16
gdt64_base:
  /* 0x00: Null descriptor */
  .quad 0x0
  /* 0x08: 64-bit code descriptor (exec/read)  - RING 0 (KERNEL CODE) */
  .quad 0x00209A0000000000
  /* 0x10: 64-bit data descriptor (read/write) - RING 0 (KERNEL DATA) */
  .quad 0x0000920000000000
  /* 0x18: 64-bit code descriptor (exec/read)  - RING 3 (USER   CODE) */
  .quad 0x0020FA0000000000
  /* 0x20: 64-bit data descriptor (read/write) - RING 3 (USER   DATA) */
  .quad 0x0000F20000000000

  /*
   * 64-bit 1 TSS per Processor is required for Long Mode.
   * to keep track of Interrupt Stack Table (only storing rsp0 (ring0) for now)
   * See Section 9.2.3 - TSS Descriptor in 64-bit mode
   */
.globl gdt64_tssd_base
gdt64_tssd_base: /* 0x28: Offset of CPU 0's TSS is 0x28 */
  .zero 16 * MAX_CPU_COUNT 

gdt64_pointer:
  .short gdt64_pointer - gdt64_base - 1
  .long gdt64_base

/*******************************************************************************
 * Interrupt Descriptor Table
 ******************************************************************************/
.globl idt_base
idt_base:
  .fill 66,8,0		# idt is uninitialized
  .fill 66,8,0		# idt is uninitialized

.globl idt_pointer
idt_pointer:
  .short  idt_pointer - idt_base - 1
  .quad   idt_base

idt_vectors:
  .quad   excep_div_by_zero
  .quad   int_default
  .quad   excep_nmi
  .quad   int_default
  .quad   excep_overflow
  .quad   int_default
  .quad   excep_invalid_opcode
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   excep_invalid_tss
  .quad   excep_seg_not_present
  .quad   excep_stack_fault
  .quad   excep_gp_fault
  .quad   excep_page_fault
  .quad   int_default
  .quad   excep_x87fpu
  .quad   excep_alignment_chk
  .quad   excep_machine_chk
  .quad   excep_simd_fp
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   int_default
  .quad   sys_handler     /* System call <- INT 0x20 */
  .quad   irq_lvt_timer   /* LAPIC Timer interrupt <- 0x21 */
  .quad   irq_handler0
  .quad   irq_handler1
  .quad   irq_handler2
  .quad   irq_handler3
  .quad   irq_handler4
  .quad   irq_handler5
  .quad   irq_handler6
  .quad   irq_handler7
  .quad   irq_handler8
  .quad   irq_handler9
  .quad   irq_handler10
  .quad   irq_handler11
  .quad   irq_handler12
  .quad   irq_handler13
  .quad   irq_handler14
  .quad   irq_handler15
  .quad   irq_handler16
  .quad   irq_handler17
  .quad   irq_handler18
  .quad   irq_handler19
  .quad   irq_handler20
  .quad   irq_handler21
  .quad   irq_handler22
  .quad   irq_handler23
  .quad   irq_handler24
  .quad   irq_handler25
  .quad   irq_handler26
  .quad   irq_handler27
  .quad   irq_handler28
  .quad   irq_handler29
  .quad   irq_handler30
  .quad   irq_handler31

/******************************************************************************
 * Task State Segment - One per CPU                                           *
 * Todo - Use this buffer when adding the support for a dual-mode kernel      *
 ******************************************************************************/
.globl tss64_base
tss64_base:
.zero 104 * MAX_CPU_COUNT 

/*******************************************************************************
 * Scheduling primitives
 ******************************************************************************/
.text
.macro RESTORE_CONTEXT
  /* pop stack watermark and check if the stack is corrupted */
  popq      %r15
  movq      $0x0123456789abcdef, %r14
  cmpq      %r14, %r15
  jne       2f
  /* Pop CR3 to switch address-spaces if neceassary */
  popq      %rax        /* Saved CR3 */
  movq      %cr3, %rbx  /* Current CR3 */
  cmpq      %rax, %rbx
  je        1f
  movq      %rax, %cr3  /* Switch the address-space */
1:
  /* pop thread's saved execution state */
  popq      %r15
  popq      %r14
  popq      %r13
  popq      %r12
  popq      %r11
  popq      %r10
  popq      %r9
  popq      %r8
  popq      %rbp
  popq      %rdi
  popq      %rsi
  popq      %rdx
  popq      %rcx
  popq      %rbx
  popq      %rax
  popf
.endm

.macro SAVE_CONTEXT
  pushf
  pushq     %rax
  pushq     %rbx
  pushq     %rcx
  pushq     %rdx
  pushq     %rsi
  pushq     %rdi
  pushq     %rbp
  pushq     %r8
  pushq     %r9
  pushq     %r10
  pushq     %r11
  pushq     %r12
  pushq     %r13
  pushq     %r14
  pushq     %r15
  movq      %cr3, %rax
  pushq     %rax
  /* push stack watermark */
  movq      $0x0123456789abcdef, %r15
  pushq     %r15
.endm

/*******************************************************************************
 * start_first_thread(TaskContext* t)
 * Sets the context of the thread pointed to by "t" and starts executing
 * it. At this point there's no thread running => no need to save the context!
 */
.globl start_first_thread
start_first_thread:
  movq      8(%rdi), %rsp       /* Retrieve new's stack pointer */
  RESTORE_CONTEXT
  retq
2:
  /* Stack consistency has failed */
  movq      %rsp, %rdi
  callq     kstack_error

/*******************************************************************************
 * switch_context(TaskContext* old, TaskContext* new, usize tss_rsp0_addr)
 */
.globl switch_context
switch_context:
/*  fxsave    48(%rdi)       TODO Save x87 FPU, MMX Technology, and SSE State */
  SAVE_CONTEXT                /* Save general x86_64 registers */
  movq      %rsp, 8(%rdi)     /* Update old's stack pointer in its TCB */
  movq      8(%rsi), %rsp     /* Retrieve new's stack pointer */
/*  fxrstor   48(%rsi)      TODO Restore x87 FPU, MMX Technology, and SSE State */
  RESTORE_CONTEXT             /* Restore general x86_64 registers */
  retq
2:
  /* Stack consistency has failed */
  movq      %rsp, %rdi
  callq     kstack_error


/*******************************************************************************
 * Initial kernel buffers (stack, paging structs, etc.)
 ******************************************************************************/
.bss
.align    0x1000
/* setup kernel's initial stack area in bss */
.globl    _KINIT_STACK_START
.comm     _KINIT_STACK_START, PER_CPU_KERNEL_STACK_SIZE * MAX_CPU_COUNT

/*
 * Kernel's Address space:
 * [0..4GB) Identity-mapped using 1GB pages (PDPT0[0..3]) as kernel's fixed map:
 * - Switching the CPUs to long-mode (64-bit)
 * - Loading any ramdisk image or kernel modules
 * - Enumeration of ACPI tables and configuring early drivers (APICs, etc.)
 * [4..8GB) Kernel's dynamic mapping area (PDPT0[4..7]) as 4KB pages for
 * - DMA mappings for drivers
 * - Temporary mappings to access physical memory
 * - etc.
 *
 * These tables are reserved here as the MMU (and some early drivers such as FB)
 * may need to start mapping into the dynamic pool before the physical memory
 * manager is initialized.
 */
/* L1) Allocate 4*512 Page Tables. Each PT can map 2MB */
.globl    kpg_tbls
.comm     kpg_tbls, 0x1000 * 512 * 4
/* L2) Allocate 4 Page Directories. Each PD can map 1GB in the kmap pool */
.globl    kpd_tlbs
.comm     kpd_tlbs, 0x1000 * 4
/* L3) Allocate 1 Page Directory Pointer Table. Each PDPT can map 512 GB*/
.globl    kpdpt0_tbl
.comm	    kpdpt0_tbl, 0x1000 /* Only first 4 PDPTEs are used */
/* L4) Allocate 1 PML4 Table. Each PML4 can map 512 TB */
.globl    kpml4_tbl
.comm	    kpml4_tbl, 0x1000 /* Only the first PML4E is used */


/******************************************************************************
 * Application Processors Startup Entry Point                                 *
 *                                                                            *
 * The Real Mode (16 bit) part of the startup procedure is marked by          *
 *_AP_STARTUP16_ENTRY and _AP_STARTUP16_END, which will be copied to 0x8000   *
 * by the BSP before APs receive the SIPI(0x8000) interrupt and start up.     *
 *                                                                            *
 * Because of how our linker script sets up the addresses above the 1MB mark  *
 * The addressing part of the instructions are manually calculated and        *
 * and adjusted for proper run-time translation of the Real Mode part.        *
 * .align is used to make the address calculation easier and less susceptible *
 * to change if a few instructions are added or removed from each sub-section *
 * of the AP startup code.                                                    *
 * Once the AP is switched over to Protected Mode (32 bits), we can rely on   *
 * the linker's address translation and the copy of the following code > 1MB  *
 ******************************************************************************/
.text
.code16
.align 64
.globl    _AP_STARTUP16_ENTRY
_AP_STARTUP16_ENTRY:                           /* <-- Runtime Address: 0x8000 */
  cli
  cld
  ljmp    $0, $0x8080 /* CS=0; jump to ap_startup_switch_to_pm */

.align 64
gdt32_base:                                    /* <-- Runtime Address: 0x8040 */
  /* Null descriptor */
  .quad 0x0
  /* 32-bit code descriptor (exec/read) */
  .quad 0x00CF9A000000FFFF
  /* 32-bit data descriptor (data/rw) */
  .quad 0x00CF92000000FFFF
gdt32_ptr:                                     /* <-- Runtime Address: 0x8058 */
  .short gdt32_ptr - gdt32_base - 1
  .long 0x8040

.align 64                                      /* <-- Runtime Address: 0x8080 */
ap_startup_switch_to_pm: 
  xorw    %ax, %ax
  movw    %ax, %ds
  lgdt    0x8058
  movl    %cr0, %eax
  orl     $1, %eax
  movl    %eax, %cr0  /* Set CR0.PE */
  ljmp    $0x8, $0x80C0  /*Switch to 32-bits and do another jump (below) */

.align 64
.code32                                        /* <-- Runtime Address: 0x80C0 */ 
  /* We are in 32-bits now but EIP is still < 1MB,
   * Calling ap_startup_32bit here jumps to the copy of this code in the kernel
   * image, i.e. > 1MB, which conforms to the linker produced addresses
   */
  jmp     ap_startup_32bit 

ap_startup_32bit:
  mov     $0x10, %ax
  mov     %ax, %ds
  mov     %ax, %es
  mov     %ax, %fs
  mov     %ax, %gs
  mov     %ax, %ss
  /* Fetch current CPU's LAPICID -> EBX[31..24] -> EDI */
  mov     $1, %eax  /* CPUID_GETFEATURES */
  cpuid
  shrl    $24, %ebx
  movl    %ebx, %edi /* <- CPU ID */
  /* Set up the CPU's RSP = _KINIT_STACK_START + (CPUID+1) * STACK_SIZE */
  inc     %edi
  movl    $PER_CPU_KERNEL_STACK_SIZE, %eax
  mul     %edi
  leal    _KINIT_STACK_START(, %eax), %esp
  /* Push the CPUID and the magic number */
  dec     %edi
  pushl   $0x5a55a400       /* Save a magic # for verification */
  pushl   %edi              /* Save the CPU LAPIC ID */
  /* Switch the AP to Long Mode using the same IDT, GDT and PML4 as BSP */
  mov $kpml4_tbl, %eax
	mov %eax, %cr3
  mov %cr4, %edx
  or  $0x20,%edx
  mov %edx, %cr4 /* Enable PAE */
  mov $0xC0000080, %ecx
  rdmsr
  or $0x00000100, %eax
  wrmsr           /* Enable Long Mode (Set IA32_EFER.LME) */
  mov %cr0, %eax
  or  $0x80000001, %eax
  mov %eax, %cr0  /* Enable PE and PG */
  lgdt gdt64_pointer
  ljmp $0x8,$1f   /* Switch to the initial 64-bit 4GB flat GDT */

.code64
1:
  /* Segment registers */
  mov     $0x10, %ax
  mov     %ax, %ds
  mov     %ax, %es
  mov     %ax, %fs
  mov     %ax, %gs
  mov     %ax, %ss
  /* Load IDT */
  lidt    idt_pointer
  /* Call into Rust */
  popq    %rdi /* Pass the CPU ID to Rust */
  callq   rust_x864_entry_ap
2:
  cli
  hlt
  jmp 2b /* should be unreachable */
.globl    _AP_STARTUP16_END
_AP_STARTUP16_END:


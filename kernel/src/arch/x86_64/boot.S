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

  /* Fixing up TSS for CPU0
   * TODO: move this to Rust and do it for all enabled CPUs
   */
  /* Flags -> RAX*/
  movq  $0x8900, %rax
  shlq  $32, %rax 
  /* TSS Base and Limit -> RCX */
  movq  $tss64_base+(0x68 * 0), %rcx
  shlq  $16, %rcx /* TSS Base */
  orq   $0x68, %rcx /* TSS Limit */
  /* First QWORD of the descriptor --> RAX */
  orq   %rcx, %rax 
  movq  %rax, gdt64_tssd_base

  /* Load the task register with the descriptor offset in GDT */
  mov   $0x28, %ax
  ltr   %ax

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
/* Zero-out all the entries in page_dir_pointer_table, pml4 */
	movl	$0, init_page_dir_pointer_table(,%ecx,4)
	movl	$0, init_pml4_table(,%ecx,4)
	incl  %ecx
.cleanup_loop_cond:
	cmpl	$1024, %ecx
	jle	.cleanup_loop_body

/* Level 4) Set pml4e[0] point to page directory pointer table (first 512 GB) */
  movl	$init_page_dir_pointer_table, %eax
  orl   $0xB, %eax                    /* Present|Writable|Cache-Write-Through */
  movl	%eax, init_pml4_table

/* Level 3) Set pdpte[0] point to the 1st GB, pdpte[1] to the 2nd GB and ...  */
  movl  $0x8B, %eax/* 0x9B: Present|Writable|Write-Through|No-Cache|HugePage */
  movl  %eax, init_page_dir_pointer_table+0
  movl  $0x4000008B, %eax
  movl  %eax, init_page_dir_pointer_table+8
  movl  $0x8000008B, %eax
  movl  %eax, init_page_dir_pointer_table+16
  movl  $0xC000008B, %eax
  movl  %eax, init_page_dir_pointer_table+24

/* Load base address of PML4 table into cr3 */
	mov $init_pml4_table, %eax
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
 * Installs a 49-item IDT table in which all items are Interrupt gates and
 * point to various interrupt entry point according to the table idt_vectors
 */
.code32
setup_idt:
  pushal
  lea idt_base,%edi
  mov $49,%ecx
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

.macro STORE_CALLER_SAVED
  pushq   %rax
  pushq   %rcx
  pushq   %rdx
  pushq   %rsi
  pushq   %rdi
  pushq   %r8
  pushq   %r9
  pushq   %r10
  pushq   %r11
.endm

.macro RESTORE_CALLER_SAVED
  popq    %r11
  popq    %r10
  popq    %r9
  popq    %r8
  popq    %rdi
  popq    %rsi
  popq    %rdx
  popq    %rcx
  popq    %rax
.endm

.macro CALL_KIRQ_HANDLER irq_num
  STORE_CALLER_SAVED
  movq    $\irq_num, %rdi
  call    kirq_handler
  RESTORE_CALLER_SAVED
.endm

int_default:
  STORE_CALLER_SAVED
  call kdefault_handler
  RESTORE_CALLER_SAVED
  iretq

excep_div_by_zero:
  STORE_CALLER_SAVED
  call kexcep_div_by_zero
  RESTORE_CALLER_SAVED
  iretq

excep_nmi:
  STORE_CALLER_SAVED
  call kexcep_nmi
  RESTORE_CALLER_SAVED
  iretq

excep_overflow:
  STORE_CALLER_SAVED
  call kexcep_overflow
  RESTORE_CALLER_SAVED
	iretq

excep_invalid_opcode:
  STORE_CALLER_SAVED
  movq %rsp, %rdi
  addq $(9*8), %rdi /* STORE_CALLER_SAVED pushed 9 registers on the stack */
  call kexcep_invalid_opcode
  RESTORE_CALLER_SAVED
	iretq

excep_invalid_tss:
  STORE_CALLER_SAVED
  call kexcep_invalid_tss
  RESTORE_CALLER_SAVED
  iretq

excep_seg_not_present:
  STORE_CALLER_SAVED
  call kexcep_seg_not_present
  RESTORE_CALLER_SAVED
  iretq

excep_stack_fault:
  STORE_CALLER_SAVED
  call kexcep_stack_fault
  RESTORE_CALLER_SAVED
  iretq

excep_gp_fault:
  STORE_CALLER_SAVED
  movq %rsp, %rdi
  addq $(9*8), %rdi /* STORE_CALLER_SAVED pushed 9 registers on the stack */
  call kexcep_gp_fault
  RESTORE_CALLER_SAVED
  iretq

excep_page_fault:
  STORE_CALLER_SAVED
  movq %rsp, %rdi
  addq $(9*8), %rdi /* STORE_CALLER_SAVED pushed 9 registers on the stack */
  call kexcep_page_fault
  RESTORE_CALLER_SAVED
  iretq

excep_x87fpu:
  STORE_CALLER_SAVED
  call kexcep_x87fpu
  RESTORE_CALLER_SAVED
  iretq

excep_alignment_chk:
  STORE_CALLER_SAVED
  call kexcep_alignment_chk
  RESTORE_CALLER_SAVED
  iretq

excep_machine_chk:
  STORE_CALLER_SAVED
  call kexcep_machine_chk
  RESTORE_CALLER_SAVED
  iretq

excep_simd_fp:
  /* Pass the value of MXCSR value as the argument to the handler */
  STORE_CALLER_SAVED
  subq    $4, %rsp
  stmxcsr (%rsp)
  movl    (%rsp), %edi
  addq    $4, %rsp
  call kexcep_simd_fp
  RESTORE_CALLER_SAVED
  iretq

sys_handler:
  STORE_CALLER_SAVED
  call    ksyscall_handler
  RESTORE_CALLER_SAVED
	iretq

irq_lvt_timer:
  STORE_CALLER_SAVED
  call    kirq_lvt_timer
  RESTORE_CALLER_SAVED
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
gdt64_tssd_base:
  /* 0x28: CPU 0 TSS */
  .quad 0x0
  .quad 0x0
  /* 0x38: CPU 1 TSS */
  .quad 0x20890000000068 
  .quad tss64_base+(0x68 * 1)
  /* 0x48: CPU 2 TSS */
  .quad 0x20890000000068 
  .quad tss64_base+(0x68 * 2)
  /* 0x58: CPU 3 TSS */
  .quad 0x20890000000068 
  .quad tss64_base+(0x68 * 3)
  /* 0x68: CPU 4 TSS */
  .quad 0x20890000000068 
  .quad tss64_base+(0x68 * 4)
  /* 0x78: CPU 5 TSS */
  .quad 0x20890000000068 
  .quad tss64_base+(0x68 * 5)
  /* 0x88: CPU 6 TSS */
  .quad 0x20890000000068 
  .quad tss64_base+(0x68 * 6)
  /* 0x98: CPU 7 TSS */
  .quad 0x20890000000068 
  .quad tss64_base+(0x68 * 7)

gdt64_pointer:
  .short gdt64_pointer - gdt64_base - 1
  .long gdt64_base

/*******************************************************************************
 * Interrupt Descriptor Table
 ******************************************************************************/
.globl idt_base
idt_base:
  .fill 50,8,0		# idt is uninitialized
  .fill 50,8,0		# idt is uninitialized

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
  jne       1f
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
1:
  /* Stack consistency has failed */
  movq      %rsp, %rdi
  callq     kstack_error

/*******************************************************************************
 * switch_context(TaskContext* old, TaskContext* new)
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
1:
  /* Stack consistency has failed */
  movq      %rsp, %rdi
  callq     kstack_error

/*******************************************************************************
 * switch_to_userspace(rdi: entrypoint, rsi: stackpointer, rdx: pml4_base, 
                       rcx: tss_rsp0_addr)
 */
.globl switch_to_userspace
switch_to_userspace:
  cli
  pushq   $(0x20 | 3)   /* Ring-3 Stack Segment */
  pushq   %rsi          /* Ring-3 Stack Pointer */
  pushq   $0x202        /* Ring-3 Starting RFLAGS */
  pushq   $(0x18 | 3)   /* Ring-3 Code Segment */
  pushq   $1f           /* Ring-3 RIP */
  mov     %rsp, (%rcx)  /* Ring-0 Stack Pointer */
  mov     %rdx, %cr3    /* Swith to the new address-space */
  iretq
1: /* In the destination Address Space and Ring-3 */
  jmp    *%rdi          /* Call the main function of the user-space process */
  hlt

/*******************************************************************************
 * Initial kernel buffers (stack, paging structs, etc.)
 ******************************************************************************/
.bss
.align    0x1000
/* setup kernel's initial stack area in bss */
.globl    _KINIT_STACK_START
.comm     _KINIT_STACK_START, PER_CPU_KERNEL_STACK_SIZE * MAX_CPU_COUNT

/*
 * Initial 4-level paging structures direct-mapping the first 4GBs used for
 * - Switching the CPUs to long-mode (64-bit)
 * - Loading any ramdisk image or kernel modules
 * - Enumeration of ACPI tables and configuring early drivers (APICs, etc.)
 *
 * Use 1 Page Directory Pointer Table with the first 4 entries directly
 * mapping to corresponding 1GB (huge page) of virt. to phys. address space.
 * Otherwise, there needs to be 2048 Page Tables (2048x4KB + 24KB).
 *
 * These structures (init_* below) can be reclaimed once the kernel starts
 * handling process address spaces.
 */
/* L1) Allocate 2048 Page Tables. Each PT can map 2MB (512x4K)
/* SKIPPED! */
/* L2) Allocate 4 Page Directories. Each PD can map 1GB */
/* SKIPPED! */
/* L3) Allocate 1 Page Directory Pointer Table. Each PDPT can map 512 GB*/
.comm	    init_page_dir_pointer_table, 0x1000 /* Only first 4 PDPTEs are used */
/* L4) Allocate 1 PML4 Table. Each PML4 can map 256 TB */
.comm	    init_pml4_table, 0x1000 /* Only the first PML4E is used */


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
  mov $init_pml4_table, %eax
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


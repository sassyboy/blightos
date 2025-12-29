/**
 * boot.S
 *
 * Description:
 * Entry-Point from a multiboot bootloader which switches the BSP processor to
 * Long mode and calls x64init() in kstub.c
 *
 * Change Log:
 * 12/25/25 - Sasan Golchin - Creation.
 */

.code32
.section .multiboot_header

.globl boot_start
boot_start:
  jmp real_start
  /* Multiboot header -- Safe to place this header in 1st page of memory for GRUB */
  .align 4
  .long 0x1BADB002 /* Multiboot magic number */
  .long 0x00000007 /* Align modules to 4KB, req. mem size, req video mode init*/
                   /* See 'info multiboot' for further info */
  .long 0xE4524FF7 /* Checksum */
  .long 0, 0, 0, 0, 0
#if STDOUT_EGA_TEXT
  .long 1          /* Graphics mode : 0=Linear Graphics, 1=EGA Text */
#elif STDOUT_VGA_RGB
  .long 0
  .long 1280, 1024, 24 /* Resolution and color depth */
#endif

.text
real_start:
  cli
  /* Setup kernel initialization stack */
  movl $stack+INIT_STACK_SIZE, %esp
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
  xor   %rdi, %rdi
  lidt  idt_pointer
  mov   %ebx, %edi /* Pass the multiboot structure as an argument */
  callq  rust_entry_x86_64
  hlt

/**
 * setup_initial_4lp
 *
 * Initializes the neceassary 4-level page tables for direct mapping of the
 * first INIT_NUM_PAGE_TABLES*2MB (up to 1GB) of the physical memory into the
 * kernel's initial virtual memory.
 */
.code32
setup_initial_4lp:
.LFB0:
	pushl	%ebp
	movl	%esp, %ebp
	pushl	%edi
	pushl	%esi
	pushl	%ebx
	subl	$44, %esp
	movl	$0, -20(%ebp)
	movl	$0, -28(%ebp)
	jmp	.L2
.L3:
/* Zero-out all the entries in page_dir, page_dir_pointer_table, pml4 */
	movl	-28(%ebp), %eax
	movl	$0, page_dir(,%eax,8)
	movl	$0, page_dir+4(,%eax,8)
	movl	-28(%ebp), %eax
	movl	$0, page_dir_pointer_table(,%eax,8)
	movl	$0, page_dir_pointer_table+4(,%eax,8)
	movl	-28(%ebp), %eax
	movl	$0, pml4_table(,%eax,8)
	movl	$0, pml4_table+4(,%eax,8)
	addl	$1, -28(%ebp)
.L2:
	cmpl	$511, -28(%ebp)
	jle	.L3
/* Level 3) Set pdpte[0] point to page directory[0] (first 1 GB) */
	movl	$page_dir_pointer_table, %eax
	movl	$0, %edx
	movl	%eax, %ebx
	andl	$-4096, %ebx
	movl	%ebx, %esi
	movl	%edx, %eax
	andl	$1048575, %eax
	movl	%eax, %edi
	movl	%esi, %eax
	orl	$7, %eax
	movl	%eax, -48(%ebp)
	movl	%edi, %eax
	orb	$0, %ah
	movl	%eax, -44(%ebp)
	movl	-48(%ebp), %eax
	movl	-44(%ebp), %edx
/* Level 4) Set pml4e[0] point to page directory pointer table (first 512 GB) */
	movl	%eax, pml4_table
	movl	%edx, pml4_table+4
	movl	$page_dir, %eax
	movl	$0, %edx
	movl	%eax, %edi
	andl	$-4096, %edi
	movl	%edi, -40(%ebp)
	movl	%edx, %eax
	andl	$1048575, %eax
	movl	%eax, -36(%ebp)
	movl	-40(%ebp), %eax
	orl	$7, %eax
	movl	%eax, -56(%ebp)
	movl	-36(%ebp), %eax
	orb	$0, %ah
	movl	%eax, -52(%ebp)
	movl	-56(%ebp), %eax
	movl	-52(%ebp), %edx
	movl	%eax, page_dir_pointer_table
	movl	%edx, page_dir_pointer_table+4
	movl	$0, -28(%ebp)
	jmp	.L4
.L7:
/* Level 2) Set page_dir to point to first num_initial_page_tables page_tables */
	movl	-28(%ebp), %eax
	sall	$12, %eax
	addl	$page_table, %eax
	andl	$-4096, %eax
	orl	$7, %eax
	movl	$0, %edx
	movl	-28(%ebp), %ecx
	movl	%eax, page_dir(,%ecx,8)
	movl	%edx, page_dir+4(,%ecx,8)
	movl	$0, -24(%ebp)
	jmp	.L5
.L6:
  /* Level 1) Setup first num_initial_page_tables page tables (each will cover 2MB) */
	movl	-20(%ebp), %eax
	andl	$-4096, %eax
	orl	$7, %eax
	movl	$0, %edx
	movl	-28(%ebp), %ecx
	movl	%ecx, %ebx
	sall	$9, %ebx
	movl	-24(%ebp), %ecx
	addl	%ebx, %ecx
	movl	%eax, page_table(,%ecx,8)
	movl	%edx, page_table+4(,%ecx,8)
	movl	$4096, %eax
	addl	%eax, -20(%ebp)
	addl	$1, -24(%ebp)
.L5:
	cmpl	$511, -24(%ebp)
	jle	.L6
	addl	$1, -28(%ebp)
.L4:
	cmpl	$INIT_NUM_PAGE_TABLES-1, -28(%ebp)
	jle	.L7

/* Load base address of PML4 table into cr3 */
	mov $pml4_table, %edx
	mov %edx, %cr3

/* Clean-up and return */
	nop
	addl	$44, %esp
	popl	%ebx
	popl	%esi
	popl	%edi
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
  movq %rsp, %rdi
  call kexcep_invalid_opcode
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
  call kexcep_gp_fault
  RESTORE_CALLER_SAVED
  iretq

excep_page_fault:
  STORE_CALLER_SAVED
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

sys_handler:
  STORE_CALLER_SAVED
  call    ksyscall_handler
  RESTORE_CALLER_SAVED
	iretq

/*******************************************************************************
 * Global Descriptor Table
 ******************************************************************************/
.data
.align 16
gdt64_base:
  /* Null descriptor */
  .quad 0x0
  /* 64-bit code descriptor (exec/read) */
  .quad 0x00209A0000000000
  /* 64-bit data descriptor (read/write) */
  .quad 0x0000920000000000

gdt64_pointer:
    .short gdt64_pointer - gdt64_base - 1
    .long gdt64_base

/*******************************************************************************
 * Interrupt Descriptor Table
 ******************************************************************************/
.globl idt_base
idt_base:
  .fill 49,8,0		# idt is uninitialized
  .fill 49,8,0		# idt is uninitialized

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
  .quad   sys_handler

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
  movq      $3, 0x18(%rdi)      /* Update status to 3 (RUNNING) */
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
  movq      $3, 0x18(%rsi)    /* Update new->status to 3 (RUNNING)*/
  movq      8(%rsi), %rsp     /* Retrieve new's stack pointer */
/*  fxrstor   48(%rsi)      TODO Restore x87 FPU, MMX Technology, and SSE State */
  RESTORE_CONTEXT             /* Restore general x86_64 registers */
  retq
1:
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
.comm     stack, INIT_STACK_SIZE
/* Setup initial 4 level pagin structures (0 to 1GB direct mapping) */
/* L1) 512 Page tables, each of 4KB */
.comm	    page_table, 0x1000 * INIT_NUM_PAGE_TABLES
/* L2) 1 Page Directory (512 PDE each of which points to a page table) */
.comm	    page_dir, 0x1000
/* L3) 1 Page Directory Pointer Table (512 PDPTE. Only the 1st one is used)*/
.comm	    page_dir_pointer_table,0x1000
/* L4) 1 PML4 Table (512 PML4E of which only the first one is used) */
.comm	    pml4_table, 0x1000

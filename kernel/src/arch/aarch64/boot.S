
/* Fake a Linux Kernel header for compatibility with QEMU and other loaders
 * https://www.kernel.org/doc/Documentation/arm64/booting.txt
 */
/* Entry point for the kernel. Registers:
 * x0 -> 32 bit pointer to DTB in memory (primary core only) / 0 (secondary cores)
 * x1 -> Reserved but we use it for the ID of the calling CPU
 * x2 -> Reserved but we use it for the stack bottom of the calling CPU
 * x3 -> Reserved
 * x4 -> 32 bit kernel entry point, boot_start location
 */

.section ".text.boot"
.globl boot_header
boot_header:
  b boot_start                /* u32 code0: Executable code */
  b boot_start                /* u32 code1: Executable code */
  .quad 0x80000              /* u64 text_offset: Image load offset, little endian */
  .quad 0 /* u64 image_size: Effective Image size, little endian */
  .quad 0                     /* u64 flags: kernel flags, little endian */
  .quad 0                     /* reserved */
  .quad 0                     /* reserved */
  .quad 0                     /* reserved */
  .long 0x644d5241;           /* Magic number, little endian, "ARM\x64" */
  .quad 0                   /* reserved (used for PE COFF offset) */

.globl boot_start
boot_start:
  /* Load the PCPU ID into x1*/
	mrs	    x1, mpidr_el1		
	and	    x1, x1, #0xFF

  /* Enable the FPU/SIMD */
  mov	x5, #0x33ff
	msr	cptr_el2, x5	/* Disable coprocessor traps to EL2 */
	msr	hstr_el2, xzr	/* Disable coprocessor traps to EL2 */
	mov	x5, #3 << 20
	msr	cpacr_el1, x5	/* Enable FP/SIMD at EL1 and EL0 */

  /* Setup the initial stack for each CPU (8KB each). x2: start of stack */
  adr     x2, early_pcpu_stack_high
  mov     x5, #8192
  mul     x5, x5, x1
  sub     x2, x2, x5
  mov     sp, x2
  mov     x6, x2

  /* Switch to EL1 for the kernel-space! */
  mrs x5, CurrentEL
  cmp x5, #0b1000  /* EL2 ? */
  beq el2_handler
  blo el1_handler

el3_handler:
  /* Secure Configuration Register (EL3)
   * RW=1: Next EL is AArch64
   * ST=1: Secure EL1 can access the Counter-timer Physical Secure Timer
   *       registers CNTPS_TVAL_EL1, CNTPS_CTL_EL1, and CNTPS_CVAL_EL1
   * EA,FIQ,IRQ=000: No interrupt/exception handling at EL3
   */
  mov     x6, #0xc00 // RW=1 | ST=1, NS=0: Secure | No IRQ/FIQ/EA handling
  msr     SCR_EL3, x6
  /* Saved Program Status Register (EL3)
   * M: AArch64_EL1h - EL1h: EL1 with SP_EL1 (stack pointer)
   * A, I, F, D: Mask SError Exception, IRQ, FIQ and Debug Exception
   */
  mov     x6, #0x3C5
  msr     SPSR_EL3, x6
  /* Exception Link Register (EL3) */
  adr     x6, el1_handler
  msr     ELR_EL3, x6
  mov     x6, sp
  ERET

el2_handler:
  /* HCR_EL2, Hypervisor Configuration Register (EL2)
   * No hypervisor support planned - Disable interrupt handling, etc
   */
  mrs x6, HCR_EL2
  and x6, x6, #~0x38 // HCR_EL2_AMO | HCR_EL2_IMO | HCR_EL2_FMO
  and x6, x6, #(~(1 << 27)) // HCR_EL2_TGE cleared: No trapping of EL0 execp.
  and x6, x6, #(~(1 << 34)) // HCR_EL2_E2H cleared: No Host OS in EL2
  msr HCR_EL2, x6

  /* Switch from EL2 to EL1
   * VERY IMPORTANT
   * Upon entring EL1 from other levels (EL2 here and EL0 later), the following
   * model is kept consistently:
   *
   * DAIF: (IRQs, FIQs, Excep, Debug) are all masked upon entering EL1 
   * EL1h: EL1 uses its own stack pointer, i.e., SP_EL1
   */
  movz x6, #0x3C5
  msr SPSR_EL2, x6

  adr x6, el1_handler
  msr ELR_EL2, x6

  mov x6, sp
  msr SP_EL1, x6
  eret

  /* Shouldn't return to EL2 anymore */

el1_handler:
  /* recover the stack pointer */
  mov sp, x6

  /* Install the interrupt vector table */
  adr     x6, aarch64_vector_table
  msr     VBAR_EL1, x6
 
  /* All CPUS: jump to Rust should not return (x1 = CPUID)*/
	bl      rust_aarch64_entry_bsp
  
halt:
	wfe
  b halt

.macro STORE_CALLER_SAVED
	stp 	  x0,   x1,   [sp, #-0x10]!
	stp 	  x2,   x3,   [sp, #-0x10]!
	stp 	  x4,   x5,   [sp, #-0x10]!
	stp     x6,   x7,   [sp, #-0x10]!
	stp 	  x8,   x9,   [sp, #-0x10]!
	stp 	  x10,  x11,  [sp, #-0x10]!
	stp 	  x12,  x13,  [sp, #-0x10]!
	stp 	  x14,  x15,  [sp, #-0x10]!
	stp 	  x16,  x17,  [sp, #-0x10]!
	stp 	  x18,  x19,  [sp, #-0x10]!
  stp 	  x29,  x30,  [sp, #-0x10]!
.endm

.macro LOAD_CALLER_SAVED
  ldp     x29,  x30,  [sp], #0x10
  ldp     x18,  x19,  [sp], #0x10
  ldp     x16,  x17,  [sp], #0x10
  ldp     x14,  x15,  [sp], #0x10
  ldp     x12,  x13,  [sp], #0x10
  ldp     x10,  x11,  [sp], #0x10
  ldp     x8,   x9,   [sp], #0x10
  ldp     x6,   x7,   [sp], #0x10
  ldp     x4,   x5,   [sp], #0x10
  ldp     x2,   x3,   [sp], #0x10
  ldp     x0,   x1,   [sp], #0x10
.endm

.balign 2048
.global aarch64_vector_table
aarch64_vector_table:
/* Sync. Exception - EL is using SP_EL0 stack  -------------------------------*/
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

/* IRQ - EL is using SP_EL0 stack --------------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_irq_el0
  LOAD_CALLER_SAVED
  eret

/* FIQ - EL is using SP_EL0 stack --------------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

/* SError - EL is using SP_EL0 stack -----------------------------------------*/
.balign 0x80
  bl aarch64_ivt_serr_el0
  eret

/* Sync. Exception - EL is using SP_ELx stack --------------------------------*/
.balign 0x80
  msr DAIFSET, #0b1111 /* No IRQ Nesting - Disable the IRQs */     
  STORE_CALLER_SAVED
  mrs x0, ESR_EL1
  mrs x1, FAR_EL1
  bl save_fpu_ctx
  bl save_el0_ctx /* Trapped EL0 task may get blocked and switched during syscall */
  bl aarch64_elx_exception
  bl rstor_el0_ctx
  bl rstor_fpu_ctx
  LOAD_CALLER_SAVED
  eret

/* IRQ - EL is using SP_ELx stack --------------------------------------------*/
.balign 0x80
  msr DAIFSET, #0b1111 /* No IRQ Nesting - Disable the IRQs */
  STORE_CALLER_SAVED
  bl save_fpu_ctx
  bl aarch64_elx_irq
  bl rstor_fpu_ctx
  LOAD_CALLER_SAVED
  eret

/* FIQ - EL is using SP_ELx stack --------------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_elx_fiq
  LOAD_CALLER_SAVED
  eret

/* SError - EL is using SP_ELx stack -----------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

/* Sync. Exception - From lower EL in AArch64 --------------------------------*/
.balign 0x80
  msr DAIFSET, #0b1111 /* No IRQ Nesting - Disable the IRQs */
  STORE_CALLER_SAVED
  bl save_fpu_ctx
  bl save_el0_ctx
  bl aarch64_ivt_excep_lower_el
  bl rstor_el0_ctx
  bl rstor_fpu_ctx
  LOAD_CALLER_SAVED
  eret

/* IRQ - From lower EL in AArch64 --------------------------------------------*/
.balign 0x80
  msr DAIFSET, #0b1111 /* No IRQ Nesting - Disable the IRQs */
  STORE_CALLER_SAVED
  bl save_fpu_ctx
  bl save_el0_ctx
  bl aarch64_ivt_irq_lower_el
  bl rstor_el0_ctx
  bl rstor_fpu_ctx
  LOAD_CALLER_SAVED
  eret

/* FIQ - From lower EL in AArch64 --------------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

/* SError - From lower EL in AArch64 -----------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

/* Synchronous Exception - From lower EL in AArch32 --------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret
  
/* IRQ - From lower EL in AArch32 --------------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

/* FIQ - From lower EL in AArch32 --------------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

/* SError - From lower EL in AArch32 -----------------------------------------*/
.balign 0x80
  STORE_CALLER_SAVED
  bl aarch64_ivt_default
  LOAD_CALLER_SAVED
  eret

save_fpu_ctx:
  STP		Q0, Q1, [SP,#-0x20]!
	STP		Q2, Q3, [SP,#-0x20]!
	STP		Q4, Q5, [SP,#-0x20]!
	STP		Q6, Q7, [SP,#-0x20]!
	STP		Q8, Q9, [SP,#-0x20]!
	STP		Q10, Q11, [SP,#-0x20]!
	STP		Q12, Q13, [SP,#-0x20]!
	STP		Q14, Q15, [SP,#-0x20]!
	STP		Q16, Q17, [SP,#-0x20]!
	STP		Q18, Q19, [SP,#-0x20]!
	STP		Q20, Q21, [SP,#-0x20]!
	STP		Q22, Q23, [SP,#-0x20]!
	STP		Q24, Q25, [SP,#-0x20]!
	STP		Q26, Q27, [SP,#-0x20]!
	STP		Q28, Q29, [SP,#-0x20]!
	STP		Q30, Q31, [SP,#-0x20]!

  mrs	  x10, fpsr
  mrs	  x11, fpcr
	stp	  x10, x11, [sp,#-0x10]!
ret

rstor_fpu_ctx:
  ldp	  x10, x11, [SP], #0x10
	msr	  fpsr, x10
	msr   fpcr, x11

  LDP		Q30, Q31, [SP], #0x20
	LDP		Q28, Q29, [SP], #0x20
	LDP		Q26, Q27, [SP], #0x20
	LDP		Q24, Q25, [SP], #0x20
	LDP		Q22, Q23, [SP], #0x20
	LDP		Q20, Q21, [SP], #0x20
	LDP		Q18, Q19, [SP], #0x20
	LDP		Q16, Q17, [SP], #0x20
	LDP		Q14, Q15, [SP], #0x20
	LDP		Q12, Q13, [SP], #0x20
	LDP		Q10, Q11, [SP], #0x20
	LDP		Q8, Q9, [SP], #0x20
	LDP		Q6, Q7, [SP], #0x20
	LDP		Q4, Q5, [SP], #0x20
	LDP		Q2, Q3, [SP], #0x20
	LDP		Q0, Q1, [SP], #0x20
ret



/* Stack pointers need to be 16-byte aligned - make room for up to 8 CPUs */
.data
.align 16
early_pcpu_stack_low:
.fill 8192,8,0
.global early_pcpu_stack_high
early_pcpu_stack_high:

/* Kernel's initial page tables
 * 4 1GB blocks - identity-mapped as normal memory for EL1
 */
.balign 1024
.global _KLVL1_PGTBL
_KLVL1_PGTBL:
  /* See Table D8-52 Stage 1 VMSAv8-64 Block and Page descriptor fields pg 7632 */
  /* V[0..4GB] -> P[0..4GB] BLOCK|MT_NORMAL|EL1-RW|INNER_SHARE|ACCESS_FLG: 0x701*/
  .quad 0x0        | 0x701
  .quad 0x40000000 | 0x701
  .quad 0x80000000 | 0x701
  .quad 0xC0000000 | 0x701
  /* V[4GB..8GB] -> Reserved for Kernel's Dynamic Mapping Pool (KMAP)         */
  /* Need 4        LVL2 tables for each 1GB region                            */
.balign 4096
.global _KLVL2_PGTBL
_KLVL2_PGTBL:
.fill 0x1000, 4, 0 
  /* Need 4 x 512 LVL3 tables for each 2MB region                             */
.global _KLVL3_PGTBL
_KLVL3_PGTBL:
.fill 0x1000, 4 * 512, 0

/*******************************************************************************
 * Scheduling primitives
 ******************************************************************************/
.text
.macro RESTORE_CONTEXT
  /* Check the watermark: 0x01234 twice */
  ldp     x0,   x1,   [sp], #0x10
  mov     x2,   #0x1234
  cmp     x0,   x2
  bne     1f
  cmp     x1,   x2
  bne     1f
  /* SPSR (flags/status) & ELR (return addr) saved prior to excep. handler */
  ldp     x0,   x1,   [sp], #0x10
  msr		  spsr_el1, x0
	msr		  elr_el1,  x1

  /* Restore the entire context */
  ldp     q30,  q31,  [sp], #0x20
  ldp     q28,  q29,  [sp], #0x20
  ldp     q26,  q27,  [sp], #0x20
  ldp     q24,  q25,  [sp], #0x20
  ldp     q22,  q23,  [sp], #0x20
  ldp     q20,  q21,  [sp], #0x20
  ldp     q18,  q19,  [sp], #0x20
  ldp     q16,  q17,  [sp], #0x20
  ldp     q14,  q15,  [sp], #0x20
  ldp     q12,  q13,  [sp], #0x20
  ldp     q10,  q11,  [sp], #0x20
  ldp     q8,   q9,   [sp], #0x20
  ldp     q6,   q7,   [sp], #0x20
  ldp     q4,   q5,   [sp], #0x20
  ldp     q2,   q3,   [sp], #0x20
  ldp     q0,   q1,   [sp], #0x20
  ldp     x30,  xzr,  [sp], #0x10
  ldp     x28,  x29,  [sp], #0x10
  ldp     x26,  x27,  [sp], #0x10
  ldp     x24,  x25,  [sp], #0x10
  ldp     x22,  x23,  [sp], #0x10
  ldp     x20,  x21,  [sp], #0x10
  ldp     x18,  x19,  [sp], #0x10
  ldp     x16,  x17,  [sp], #0x10
  ldp     x14,  x15,  [sp], #0x10
  ldp     x12,  x13,  [sp], #0x10
  ldp     x10,  x11,  [sp], #0x10
  ldp     x8,   x9,   [sp], #0x10
  ldp     x6,   x7,   [sp], #0x10
  ldp     x4,   x5,   [sp], #0x10
  ldp     x2,   x3,   [sp], #0x10
  ldp     x0,   x1,   [sp], #0x10

.endm

.macro SAVE_CONTEXT
	/* Save the entire context. Not efficient but easy! */
	stp 	  x0,   x1,   [sp, #-0x10]!
	stp 	  x2,   x3,   [sp, #-0x10]!
	stp 	  x4,   x5,   [sp, #-0x10]!
	stp     x6,   x7,   [sp, #-0x10]!
	stp 	  x8,   x9,   [sp, #-0x10]!
	stp 	  x10,  x11,  [sp, #-0x10]!
	stp 	  x12,  x13,  [sp, #-0x10]!
	stp 	  x14,  x15,  [sp, #-0x10]!
	stp 	  x16,  x17,  [sp, #-0x10]!
	stp 	  x18,  x19,  [sp, #-0x10]!
	stp 	  x20,  x21,  [sp, #-0x10]!
	stp 	  x22,  x23,  [sp, #-0x10]!
	stp 	  x24,  x25,  [sp, #-0x10]!
	stp 	  x26,  x27,  [sp, #-0x10]!
	stp 	  x28,  x29,  [sp, #-0x10]!
	stp 	  x30,  xzr,  [sp, #-0x10]!
	stp		  q0,   q1,   [sp, #-0x20]!
	stp		  q2,   q3,   [sp, #-0x20]!
	stp		  q4,   q5,   [sp, #-0x20]!
	stp		  q6,   q7,   [sp, #-0x20]!
	stp		  q8,   q9,   [sp, #-0x20]!
	stp		  q10,  q11,  [sp, #-0x20]!
	stp	  	q12,  q13,  [sp, #-0x20]!
	stp		  q14,  q15,  [sp, #-0x20]!
	stp		  q16,  q17,  [sp, #-0x20]!
	stp		  q18,  q19,  [sp, #-0x20]!
	stp		  q20,  q21,  [sp, #-0x20]!
	stp		  q22,  q23,  [sp, #-0x20]!
	stp		  q24,  q25,  [sp, #-0x20]!
	stp		  q26,  q27,  [sp, #-0x20]!
	stp		  q28,  q29,  [sp, #-0x20]!
	stp		  q30,  q31,  [sp, #-0x20]!
	/* SPSR (flags/status) & ELR (return addr) saved prior to excep. handler */
	mrs		  x20,   spsr_el1
	mrs		  x21,   elr_el1
	stp 	  x20,   x21,   [sp, #-0x10]!
  /* Stack watermark: 0x1234 twice */
  movz    x20,   #0x1234
  stp     x20,   x20,   [sp, #-0x10]!
.endm

/* For interrupts/exception from EL0 we should also save/restore SPSR_EL1,
 * ELR_EL1 (last status and ret-address of EL0), SP_EL0 and TTBR1_EL1 
 * (last EL0 stack pointer and last user address-space). This is because the
 * exception handler may end up switching to another task and overwriting these
 * registers.
 * E.g., Spawn or Sleep operations of the TaskControl system call!
 * Note that SAVE/RESTOR_CONTEXT don't need to do that as they always take place
 * in EL1, and should the scheduler select a user-space task, RESTORE_CONTEXT
 * ends up taking the return from an EL0 interrupt/exception handler where the
 * task last entered the kernel.
 */
save_el0_ctx:
  mrs     x10,  sp_el0
  mrs     x11,  ttbr1_el1
  stp 	  x10,  x11,  [sp, #-0x10]!

  mrs     x10,  spsr_el1
  mrs     x11,  elr_el1
  stp 	  x10,  x11,  [sp, #-0x10]!
ret

rstor_el0_ctx:
  ldp     x10,  x11,  [sp], #0x10
  msr     spsr_el1, x10
  msr     elr_el1,  x11

  ldp 	  x10,  x11,  [sp], #0x10
  msr     sp_el0, x10
  dsb     sy
  tlbi    vmalle1
  msr     ttbr1_el1, x11
  isb
ret

/*******************************************************************************
 * start_first_thread(TaskContext* t)
 * Sets the context of the thread pointed to by "t" and starts executing
 * it. At this point there's no thread running => no need to save the context!
 */
.globl start_first_thread
start_first_thread:
  ldr     x1,   [x0, #8]    /* Retrieve new's stack pointer */
  mov     sp,   x1
  RESTORE_CONTEXT
  eret                      /* use eret to enable the interrupts! */
1:
  /* Stack consistency has failed */
  mov     x0,   sp
  bl      aarch64_cs_error

/*******************************************************************************
 * switch_context(TaskContext* old, TaskContext* new)
 */
.globl switch_context
switch_context:
  dsb sy
  isb sy
  SAVE_CONTEXT
  mov     x6,     sp
  str     x6,     [x0, #8]    /* Update old's stack pointer in its TCB */
  ldr     x6,     [x1, #8]    /* Retrieve new's stack pointer */
  mov     sp,     x6
  dsb sy
  isb sy
  RESTORE_CONTEXT
  ret
1:
  /* Stack consistency has failed */
  mov     x0,   sp
  bl      aarch64_cs_error

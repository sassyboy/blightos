# OBJECT FILES LIST ###########################################################
OBJFILES= \
src/arch/x86_64/boot.o

# GCC OPTIMIZATION LEVEL #######################################################
CFLAGS += -o2
# CFLAGS += -g

# STDIO CONFIGURATION ##########################################################
CFLAGS += -DSTDOUT_EGA_TEXT
# CFG += -DSTDOUT_VGA_RGB
# CFG += -DSTDOUT_UART

# KERNEL STARTUP OPTIONS #######################################################

# Kernel's Initial STACK size in bytes
# Should be enough before the kernel starts multi-processing
CFLAGS += -DINIT_STACK_SIZE=4096

# Number of initial page tables required to hold kernel's code and data + the
# init-ramdisk module which is loaded right after kernel by the bootloader.
# Each page table covers 2 MB (Max # Tables: 512).
# From 0x0 to INIT_NUM_PAGE_TABLES*2MB is identity-mapped by boot.S
CFLAGS += -DINIT_NUM_PAGE_TABLES=4

# SELECT ONE OF THE SCHEDULERS BELOW ###########################################

# 1) First-Come First-Served (non-preemptive)
# CFG += -DSCHED_FCFS

# 2) Round-Robin (Preemptive - Every Quantum = 1 kernel tick = 250us)
CFLAGS += -DSCHED_RR
CFLAGS += -DSCHED_RR_QUANTUM=100

# SELECT DEBUGGING MESSAGES ####################################################
# Debug messages from kernel.c
CFLAGS += -DKDEBUG

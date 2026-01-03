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
MAX_CPU_COUNT=8
CFLAGS += -DMAX_CPU_COUNT=$(MAX_CPU_COUNT)
LFLAGS= --defsym MAX_CPU_COUNT=$(MAX_CPU_COUNT)
# Kernel's Initial STACK size in bytes for each CPU: 8KB
# Should be enough before the kernel starts multi-processing
CFLAGS += -DPER_CPU_KERNEL_STACK_SIZE=8192

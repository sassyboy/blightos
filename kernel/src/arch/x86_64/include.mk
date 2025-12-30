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


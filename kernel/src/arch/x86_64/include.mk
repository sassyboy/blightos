# OBJECT FILES LIST ###########################################################
OBJFILES= \
src/arch/x86_64/boot.o

# RUST SOURCE LIST #############################################################
ARCH_RST_SRC= \
src/arch/x86_64/stub.rs \
src/arch/x86_64/mmu.rs \
src/arch/x86_64/systimer.rs 

# GCC OPTIMIZATION LEVEL #######################################################
CFLAGS += -o2
# CFLAGS += -g

# STDIO CONFIGURATION ##########################################################
# CFLAGS += -DSTDOUT_EGA_TEXT
CFLAGS += -DSTDOUT_VGA_RGB -DVGA_RGB_WIDTH=1280 -DVGA_RGB_HEIGHT=1024
# CFLAGS += -DSTDOUT_VGA_RGB -DVGA_RGB_WIDTH=1024 -DVGA_RGB_HEIGHT=768
# CFLAGS += -DSTDOUT_VGA_RGB -DVGA_RGB_WIDTH=800 -DVGA_RGB_HEIGHT=600
# CFLAGS += -DSTDOUT_UART

# KERNEL STARTUP OPTIONS #######################################################
MAX_CPU_COUNT=8
CFLAGS += -DMAX_CPU_COUNT=$(MAX_CPU_COUNT)
LFLAGS= --defsym MAX_CPU_COUNT=$(MAX_CPU_COUNT)
# Kernel's Initial STACK size in bytes for each CPU: 8KB
# Should be enough before the kernel starts multi-processing
CFLAGS += -DPER_CPU_KERNEL_STACK_SIZE=8192

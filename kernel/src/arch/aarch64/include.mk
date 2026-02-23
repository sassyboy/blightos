# OBJECT FILES LIST ###########################################################
OBJFILES= \
src/arch/aarch64/boot.o

# RUST SOURCE LIST #############################################################
ARCH_RST_SRC= \
src/arch/aarch64/stub.rs \
src/arch/aarch64/fdt.rs \
src/arch/aarch64/systimer.rs \
src/arch/aarch64/mmu.rs

# KERNEL STARTUP OPTIONS #######################################################
MAX_CPU_COUNT=8
CFLAGS += -DMAX_CPU_COUNT=$(MAX_CPU_COUNT)
LFLAGS= --defsym MAX_CPU_COUNT=$(MAX_CPU_COUNT)
# Kernel's Initial STACK size in bytes for each CPU: 8KB
# Should be enough before the kernel starts multi-processing
CFLAGS += -DPER_CPU_KERNEL_STACK_SIZE=8192
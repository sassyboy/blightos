ARCH=x86_64
TARGET=$(ARCH)-unknown-none
PROFILE=release
#
# rustc options:
# -O: optimizes for --release and avoids some debug calls/instrumentations
# --emit=link --crate-type=staticlib: generates a static library with
#                                     all compiler internals necessary
# --target=x86_64-unknown-none: bare-metal x86_64 compiler target/internals
#
RC=cargo build -Zbuild-std=core,alloc

ifeq ($(findstring all,$(DEBUG)),all)
	FEATURES+=debug_arch,debug_pmm,debug_heap
else
	ifeq ($(findstring arch,$(DEBUG)),arch)
		FEATURES+=debug_arch,
	endif
	ifeq ($(findstring pmm,$(DEBUG)),pmm)
		FEATURES+=debug_pmm,
	endif
	ifeq ($(findstring heap,$(DEBUG)),heap)
		FEATURES+=debug_heap,
	endif
	ifeq ($(findstring sched,$(DEBUG)),sched)
		FEATURES+=debug_sched,
	endif
endif

#To generate Debug info: make FEATURES="debug_arch"
ifdef FEATURES
	RCFEATS=--features "$(FEATURES)"
else
	RCFEATS=
endif
CC=$(TOOLCHAIN_PREFIX)gcc
LD=$(TOOLCHAIN_PREFIX)ld
OBJCPY=$(TOOLCHAIN_PREFIX)objcopy

# Prepare the list of the object files to compile into the kernel:
# 1) Add the platform-dependent object files are already added by the
# corresponding include.mk
include src/arch/$(ARCH)/include.mk
# 2) Add the platform-agnostic parts of the kernel


BUILDDIR=../build/
KERN_RST=../target/$(TARGET)/$(PROFILE)/libkernel.a
KERN_ELF=$(BUILDDIR)kernel.elf
KERN_RST_SRC= \
		src/kernel.rs \
		src/util.rs	\
		src/arch/x86_64/stub.rs \
		src/arch//x86_64/vga.rs	\
		src/arch/mod.rs	\
		src/mem/mod.rs	\
		src/mem/phys.rs	\
		src/mem/virt.rs	\
		src/mem/heap.rs	\
		src/sched/mod.rs \
		src/sched/sched_rr.rs \
		src/drivers/mod.rs

all: $(KERN_ELF)

$(KERN_ELF): $(OBJFILES) $(KERN_RST)
	@mkdir -p $(BUILDDIR)
	$(LD) -T src/arch/$(ARCH)/link.ld $(LFLAGS) -o $@ $^

$(KERN_RST): $(KERN_RST_SRC)
	$(RC) --$(PROFILE) --target="$(TARGET)" $(RCFEATS)

%.o: %.S
	$(CC) -c $(CFLAGS) $< -o $@

clean:
	cargo clean
	rm -rf $(OBJFILES) $(KERN_ELF) $(BOOT_IMG)

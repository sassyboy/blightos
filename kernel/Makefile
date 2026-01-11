PLATDIR=x86_64
#
# rustc options:
# -O: optimizes for --release and avoids some debug calls/instrumentations
# --emit=link --crate-type=staticlib: generates a static library with
#                                     all compiler internals necessary
# --target=x86_64-unknown-none: bare-metal x86_64 compiler target/internals
#
RC=cargo build

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
include src/arch/$(PLATDIR)/include.mk
# 2) Add the platform-agnostic parts of the kernel


BUILDDIR=../build/
KERN_RST=$(BUILDDIR)libkernel.a
KERN_ELF=$(BUILDDIR)kernel.elf

all: $(KERN_ELF)

$(KERN_ELF): $(OBJFILES) $(KERN_RST)
	$(LD) -T src/arch/$(PLATDIR)/link.ld $(LFLAGS) -o $@ $^

$(KERN_RST): src/kernel.rs
	$(RC) --release --target="x86_64-unknown-none" $(RCFEATS)
	mv ../target/x86_64-unknown-none/release/libkernel.a $(KERN_RST)
	rm -rf ../target

%.o: %.S
	$(CC) -c $(CFLAGS) $< -o $@

clean:
	cargo clean
	rm -rf $(OBJFILES) $(KERN_RST) $(KERN_ELF) $(BOOT_IMG)

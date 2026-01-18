PLAT=x86_64
PROGRAM=shell
RC=cargo build


#To generate Debug info: make FEATURES="debug_arch"
ifdef FEATURES
	RCFEATS=--features "$(FEATURES)"
else
	RCFEATS=
endif
CC=$(TOOLCHAIN_PREFIX)gcc
LD=$(TOOLCHAIN_PREFIX)ld
LFLAGS=


RTLIB_OBJS=../rtlib/$(PLAT)-stub.o
RTLIB_LINK=../rtlib/$(PLAT).ld
BUILDDIR=../../build/
PROGRAM_RST=$(BUILDDIR)lib$(PROGRAM).a 
PROGRAM_ELF=$(BUILDDIR)$(PROGRAM).elf


all: $(PROGRAM_ELF)

$(PROGRAM_ELF): $(RTLIB_OBJS) $(PROGRAM_RST)
	$(LD) -T $(RTLIB_LINK) $(LFLAGS) -o $@ $^

$(PROGRAM_RST): main.rs
	$(RC) --release --target="x86_64-unknown-none"
	mv ../../target/x86_64-unknown-none/release/lib$(PROGRAM).a $(PROGRAM_RST)
	rm -rf ../../target

%.o: %.S
	$(CC) -c $(CFLAGS) $< -o $@

clean:
	cargo clean
	rm -rf $(RTLIB_OBJS) $(PROGRAM_RST) $(PROGRAM_ELF)
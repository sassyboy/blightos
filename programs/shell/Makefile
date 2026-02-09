ARCH=x86_64
TARGET=$(ARCH)-unknown-none
PROFILE=release
PROGRAM=shell
RC=cargo build -Zbuild-std=core,alloc

#To generate Debug info: make FEATURES="debug_arch"
ifdef FEATURES
	RCFEATS=--features "$(FEATURES)"
else
	RCFEATS=
endif
CC=$(TOOLCHAIN_PREFIX)gcc
LD=$(TOOLCHAIN_PREFIX)ld
LFLAGS=


RTLIB_OBJS=../rtlib/$(ARCH)-stub.o
RTLIB_RS_SRC= \
		../rtlib/src/lib.rs \
		../rtlib/src/syscall.rs \
		../rtlib/src/stdio.rs \
		../rtlib/src/fileio.rs \
		../rtlib/src/fileio.rs 

RTLIB_LINK=../rtlib/$(ARCH).ld
BUILDDIR=../../build/
PROGRAM_RST=../../target/$(TARGET)/$(PROFILE)/lib$(PROGRAM).a
PROGRAM_ELF=$(BUILDDIR)$(PROGRAM).elf


all: $(PROGRAM_ELF)

$(PROGRAM_ELF): $(RTLIB_OBJS) $(PROGRAM_RST) $(RTLIB_RS_SRC)
	@mkdir -p $(BUILDDIR)
	$(LD) -T $(RTLIB_LINK) $(LFLAGS) -o $@ $(RTLIB_OBJS) $(PROGRAM_RST)

$(PROGRAM_RST): main.rs
	$(RC) --$(PROFILE) --target="$(TARGET)"

%.o: %.S
	$(CC) -c $(CFLAGS) $< -o $@

clean:
	cargo clean
	rm -rf $(RTLIB_OBJS) $(PROGRAM_ELF)

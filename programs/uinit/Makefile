PROGRAM=uinit

include ../../config.mk
include ../rtlib/include.mk

TARGET=$(ARCH)-unknown-none
PROFILE=release
RC=cargo build -Zbuild-std=core,alloc

#To generate Debug info: make FEATURES="debug_arch"
ifdef FEATURES
	RCFEATS=--features "$(FEATURES)"
else
	RCFEATS=
endif
CC=$(C_TOOLCHAIN_PREFIX)gcc
LD=$(C_TOOLCHAIN_PREFIX)ld
LFLAGS=


BUILDDIR=../../build/
PROGRAM_RST=../../target/$(TARGET)/$(PROFILE)/lib$(PROGRAM).a
PROGRAM_ELF=$(BUILDDIR)$(PROGRAM).box


all: $(PROGRAM_ELF)

$(PROGRAM_ELF): $(RTLIB_OBJS) $(PROGRAM_RST)
	@mkdir -p $(BUILDDIR)
	$(LD) -T $(RTLIB_LINK) $(LFLAGS) -o $@ $(RTLIB_OBJS) $(PROGRAM_RST)

$(PROGRAM_RST): $(RTLIB_RS_SRC) main.rs
	$(RC) --$(PROFILE) --target="$(TARGET)"

%.o: %.S
	$(CC) -c $(CFLAGS) $< -o $@

clean:
	cargo clean
	rm -rf $(RTLIB_OBJS) $(PROGRAM_ELF)

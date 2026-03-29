RTLIB_RS_SRC= \
		../rtlib/src/lib.rs \
		../rtlib/src/env.rs \
		../rtlib/src/syscall.rs \
		../rtlib/src/stdio.rs \
		../rtlib/src/fileio.rs \
		../rtlib/src/task.rs \
		../rtlib/src/heap.rs \
		../rtlib/src/zlib.rs \
		../rtlib/src/audio/mod.rs \
		../rtlib/src/audio/wav.rs \
		../rtlib/src/audio/beeper.rs \
		../rtlib/src/graphics/mod.rs \
		../rtlib/src/graphics/framebuffer.rs \
		../rtlib/src/graphics/png.rs \

RTLIB_OBJS=../rtlib/$(ARCH)-stub.o
RTLIB_LINK=../rtlib/$(ARCH).ld

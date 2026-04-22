RTLIB_RS_SRC= \
		../rtlib/src/lib.rs \
		../rtlib/src/env.rs \
		../rtlib/src/time.rs \
		../rtlib/src/syscall.rs \
		../rtlib/src/stdio.rs \
		../rtlib/src/fileio.rs \
		../rtlib/src/task.rs \
		../rtlib/src/heap.rs \
		../rtlib/src/zlib.rs \
		../rtlib/src/audio/mod.rs \
		../rtlib/src/audio/wav.rs \
		../rtlib/src/audio/beeper.rs \
		../rtlib/src/graphics/font.rs \
		../rtlib/src/graphics/mod.rs \
		../rtlib/src/graphics/framebuffer.rs \
		../rtlib/src/graphics/png.rs \
		../rtlib/src/gui/button.rs \
		../rtlib/src/gui/imagebox.rs \
		../rtlib/src/gui/label.rs \
		../rtlib/src/gui/list.rs \
		../rtlib/src/gui/menu.rs \
		../rtlib/src/gui/mod.rs \
		../rtlib/src/gui/label.rs \
		../rtlib/src/gui/textedit.rs \
		../rtlib/src/gui/theme.rs \
		../rtlib/src/gui/window.rs \
		../rtlib/src/hid/mod.rs \

RTLIB_OBJS=../rtlib/$(ARCH)-stub.o
RTLIB_LINK=../rtlib/$(ARCH).ld

RTLIB_RS_SRC= \
		../rtlib/src/lib.rs \
		../rtlib/src/syscall.rs \
		../rtlib/src/stdio.rs \
		../rtlib/src/fileio.rs \
		../rtlib/src/task.rs \
		../rtlib/src/heap.rs \

RTLIB_OBJS=../rtlib/$(ARCH)-stub.o
RTLIB_LINK=../rtlib/$(ARCH).ld

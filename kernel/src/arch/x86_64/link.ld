OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)
_KERNEL_START = 0x100000 + SIZEOF_HEADERS;

ENTRY(boot_start)
SECTIONS {
  /* Bootstrap physical section, at virtual and physical address _KERNEL_START */
  . = _KERNEL_START;

  . = ALIGN(8);
  _KERNEL_TEXT_START = .;
  .text : {
    *(.multiboot_header)
    *(.text)
  }
  _KERNEL_TEXT_END = .;

  . = ALIGN(8);
  _KERNEL_RODATA_START = .;
  .rodata : { *(.rodata) }
  _KERNEL_RODATA_END = .;

  . = ALIGN(8);
  _KERNEL_DATA_START = .;
  .data : { *(.data) }
  _KERNEL_DATA_END = .;

  . = ALIGN(8);
  _KERNEL_BSS_START = .;
  .bss : { *(.bss COMMON) }
  _KERNEL_BSS_END = .;

  . = ALIGN(8);
  _KERNEL_PERCPU_START = .;
  _KERNEL_PERCPU_END   = _KERNEL_PERCPU_START + SIZEOF(.percpu);
  .percpu 0x0 : AT(_KERNEL_PERCPU_START) {
    *(.percpu_global)
    . = ALIGN(8);
    _KERNEL_PERCPU_SIZE = .;
    . = _KERNEL_PERCPU_SIZE * MAX_CPU_COUNT;
  }
  . = _KERNEL_PERCPU_END;

  _KERNEL_END = . ;
}

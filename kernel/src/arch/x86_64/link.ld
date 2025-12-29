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

  . = ALIGN(0x1000);
  .eh_frame (NOLOAD) : { *(.eh_frame) } : NONE

  _KERNEL_END = . ;
}

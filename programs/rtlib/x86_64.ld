OUTPUT_FORMAT("elf64-x86-64")

/* For now we assume the user-space starts at 8GB virtual */
_USERSPACE_START = 0x200000000 + SIZEOF_HEADERS;

ENTRY(_start)
SECTIONS {
  . = _USERSPACE_START;

  . = ALIGN(8);
  _USER_TEXT_START = .;
  .text : {
    *(.text)
  }
  _USER_TEXT_END = .;

  . = ALIGN(8);
  _USER_RODATA_START = .;
  .rodata : { *(.rodata) }
  _USER_RODATA_END = .;

  . = ALIGN(8);
  _USER_DATA_START = .;
  .data : { *(.data) }
  _USER_DATA_END = .;

  . = ALIGN(8);
  _USER_BSS_START = .;
  .bss : { *(.bss COMMON) }
  _USER_BSS_END = .;


  _USER_END = . ;
}

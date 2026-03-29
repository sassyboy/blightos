.text
.code64
.globl _start
_start:
    call _main_stub
ret /* The OS has put the return address for us! */
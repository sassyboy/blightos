.text
.code64
.globl _start
_start:
    call main
ret /* The OS has put the return address for us! */
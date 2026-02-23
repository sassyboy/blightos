.text
.globl _start
_start:
    bl main
halt:
    wfe
    b halt
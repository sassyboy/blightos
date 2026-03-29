.text
.globl _start
_start:
    bl _main_stub
halt:
    wfe
    b halt
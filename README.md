# BlightOS
A lifelong C developer trying out Rust!

Install rust and the nightly `x86_64-unknown-none` toolchain, `gcc`,
`qemu-system-x86_64`, and `grub-mkrescue` on your Linux dev machine, and run:
- `make` or
- `make DEBUG=yes` to add debug output in the arch-dependent code.


---
## Features
- Minimal x86_64 (64-bit/Long Mode) architecture support 
- Basic 25x80 EGA-Text as the early/startup console
- Legacy PIC and PIT support for now (will be replaced by APIC & HPET)
- Minimal/Sad Round Robin task scheduler
- Minimal ACPI support for SMP enumeration
- IOAPIC/LAPIC support (In progress...)
- Symmetric Multiprocessing (In progress...)
---
## Screenshot
![Screenshot](https://github.com/sassyboy/blightos/blob/main/screenshot.png)

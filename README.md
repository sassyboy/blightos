# BlightOS
A lifelong C developer trying out Rust!

Install rust and the nightly `x86_64-unknown-none` toolchain, `gcc`,
`qemu-system-x86_64`, and `grub-mkrescue` on your Linux dev machine, and run:
- `make` or
- `make DEBUG=yes` to add debug output in the arch-dependent code.


---
## Features
- Legacy BIOS and UEFI Multiboot-2 support
- Minimal x86_64 (64-bit/Long Mode) architecture support
- Minimal ACPI support for SMP enumeration
- Symmetric Multiprocessing (SMP)
- IOAPIC and LAPIC support for SMP IRQ routing and per-cpu task preemption
- Local FSFC and RR task schedulers
- Flat 4GB PML-4 virtual memory
- Bitmap-based physical memory allocator
- Basic VESA Graphics support with built-in bitmap font rendering
---
## Screenshot
![Screenshot](https://github.com/sassyboy/blightos/blob/main/screenshot.png)

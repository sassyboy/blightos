# BlightOS
A lifelong C developer trying out Rust!

Install rust and the nightly `x86_64-unknown-none` toolchain, `gcc`,
`qemu-system-x86_64`, and `grub-mkrescue` on your Linux dev machine, and run:
- `make` for a release build
- `make DEBUG=all` or `make DEBUG=comp1,comp2,etc` for building with additional
debugging information (directed at the default serial port) pertaining to all or
any combination of components. Available debug options are:
    - `arch`: Architecture-dependent module (arch/x86_64/stub.rs).
    - `pmm` : Physical memory allocator (mem/pmm.rs)
    - `heap`: Dynamic memory allocator (mem/heap.rs)

---
## Features
- Legacy BIOS and UEFI Multiboot-2 support
- Minimal x86_64 (64-bit/Long Mode) architecture support
- Minimal ACPI support for SMP enumeration
- Symmetric Multiprocessing (SMP)
- IOAPIC and LAPIC support for SMP IRQ routing and per-cpu task preemption
- Local FCFS and RR task schedulers
- Flat 4GB PML-4 virtual memory
- Bitmap physical memory allocator
- Simplified TLSF heap allocator
- Basic VESA Graphics support with built-in bitmap font rendering
---
## Screenshot
![Screenshot](https://github.com/sassyboy/blightos/blob/main/screenshot.png)

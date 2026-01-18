# BlightOS
A lifelong C developer trying out Rust!

Install rust and the nightly `x86_64-unknown-none` toolchain, `gcc`,
`qemu-system-x86_64`, and `grub-mkrescue` on your Linux dev machine, and run:
- `make` for a release build
- `make DEBUG=all` or `make DEBUG=comp1,comp2,etc` for building with additional
debugging information (directed at the default serial port) pertaining to all or
any combination of components. Available debug options are:
    - `arch` : Architecture-dependent module (arch/x86_64/stub.rs)
    - `pmm`  : Physical memory allocator (mem/pmm.rs)
    - `heap` : Dynamic memory allocator (mem/heap.rs)
    - `sched`: Task scheduler events

---
## Features
- Kernel:
    - Architecture: Dual-mode monolithic (Ring3: user-space/ Ring0: kernel-space)
    - H/W Architecture: x86_64 Long Mode (64-bit)
    - Startup: BIOS/UEFI Multiboot2-compatible bootloader required
    - Minimal ACPI support for SMP enumeration
    - Symmetric Multiprocessing (SMP) up to 8 CPUs
    - Local Round-Robin and FCFS task schedulers
    - Bitmap physical memory allocator
    - Flat 4GB PML-4 virtual memory
    - Simplified TLSF heap allocator
- Device support:
    - IOAPIC/LAPIC for SMP IRQ routing and per-cpu task preemption
    - Basic VESA Graphics support with built-in bitmap font rendering
    - Basic i8046 PS/2 Keyboard driver
- User-space runtime library
    - Basic formatted std-output and std-readline syscall wrappers

---
## Screenshot
![Screenshot](https://github.com/sassyboy/blightos/blob/main/screenshot.png)

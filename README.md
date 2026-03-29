# BlightOS
A lifelong C developer trying out Rust!

Install rust and the nightly `x86_64-unknown-none` toolchain, `gcc`,
`qemu-system-x86_64`, and `grub-mkrescue` on your Linux dev machine, and run:
- `make` for a release build
- `make DEBUG=all` or `make DEBUG=comp1,comp2,etc` for building with additional
debugging information (directed at the default serial port) pertaining to all or
any combination of components. Available debug options are:
    - `arch` : Architecture-dependent module (arch/x86_64/stub.rs)
    - `kern` : Kernel's main entry (init, syscall handling, etc in kernel.rs)
    - `pmm`  : Physical memory allocator (mem/pmm.rs)
    - `heap` : Dynamic memory allocator (mem/heap.rs)
    - `sched`: Task scheduler events
    - etc. (See `kernel/Makefile` for more)
---
## Features
- Kernel:
    - Architecture: Dual-mode monolithic
    - H/W Architecture (selectd in `config.mk`):
        - x86_64 Long Mode (64-bit) - Tested on QEMU and real hardware
        - Aarch64 (ARMv8) - Only tested on QEMU (raspi3)
    - Startup:
        - x86_64: BIOS/UEFI Multiboot2-compatible bootloader required
        - Aarch64:Linux-capable loader + device tree blob
    - Minimal ACPI support for SMP enumeration, reboot, etc.
    - Symmetric Multiprocessing (SMP) up to 8 CPUs
    - Local Round-Robin scheduling w\ automatic load balancing, task migration, etc.
    - Bitmap physical memory allocator
    - PML-4 virtual memory manager
    - Simplified TLSF heap allocator
    - Unix-line Virtual File System
        - `disk#.#:/` for normal file system access using a `disk.partition:` pattern
        - `driver-name:/` for accessing a specific driver via VFS. Check out `machine:/` for example
    - ELFBinary loader
        - The first supported partition (FAT) that presents the path `/blightos/shell.elf` would be considered root.
- Device support:
    - IOAPIC/LAPIC for SMP IRQ routing and per-cpu task preemption
    - Graphics (minimal fb support + built-in bmp font):
        - x86_64: Basic VESA Graphics
        - Aarch64: Broadcom VideoCore IV (BCM2835)
    - Audio (Intel HDA):
        - Dual buffering for single stream of stereo audio @ 48KHz, 16bps
    - Basic i8046 PS/2 Keyboard driver
    - AHCI (SATA) Bus Controller (Read-only)
    - eMMC (SDCard) Controller (Read-only)
    - FAT12/16/32 File System Driver (Read-only)
- User-space runtime library
    - Basic stdio,fileio and syscall wrappers
    - FF Heap allocator (explicit free-list + coalescing)
    - ZLIB decoder
    - PNG file loader
    - WAV file loader and Waveform/Note generator

---
## Screenshot
![Screenshot](https://github.com/sassyboy/blightos/blob/main/screenshot.png)

---
## Installation Steps on Ubuntu
### Toolchain
- Install rust:
`curl --proto '=https' --tlsv1.3 https://sh.rustup.rs -sSf | sh`
- Once the installation is complete, you may need to reload your shell's PATH environment variable. You can do this by running:
`source "$HOME/.cargo/env"`
- Add the x64 bare-metal development target `x86_64-unknown-none` to your rust environemt, and install the nightly toolchain to use the target:
```
rustup target add x86_64-unknown-none
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly-x86_64-unknown-none
```
- Add the AArch64 bare-metal target `aarch64-unknown-none`:
```
rustup target add aarch64-unknown-none
```
- Boot disk image creation tools:
`sudo apt install grub-pc-bin xorriso`

- Emulation environment (QEMU+KVM)
`sudo apt install qemu-kvm libvirt-daemon-system bridge-utils virt-manager virtinst libvirt-clients -y`

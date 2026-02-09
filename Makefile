BUILDDIR=build/
KERN_ELF=$(BUILDDIR)kernel.elf
SHELL_ELF=$(BUILDDIR)shell.elf
GRUB_CFG=$(BUILDDIR)grub.cfg
BOOT_IMG=$(BUILDDIR)boot.img
DISK_IMG=$(BUILDDIR)disk.img


all: kernel $(BOOT_IMG) run

kernel: force
	@echo "=============================================================="
	@echo "==                   Building the Kernel                    =="
	@echo "=============================================================="
	make -C kernel all

$(SHELL_ELF): force
	@echo "=============================================================="
	@echo "==                Building the Shell Program                =="
	@echo "=============================================================="
	make -C programs/shell all

$(BOOT_IMG): $(KERN_ELF) $(SHELL_ELF) $(GRUB_CFG)
	@echo "=============================================================="
	@echo "==  Createing a GRUB Resuce Disk Image to boot the Kernel   =="
	@echo "=============================================================="
	mkdir -p $(BUILDDIR)/isofiles/boot/grub
	cp $(KERN_ELF) $(BUILDDIR)/isofiles/boot/kernel.elf
	cp $(SHELL_ELF) $(BUILDDIR)/isofiles/boot/shell.elf
	cp $(GRUB_CFG) $(BUILDDIR)/isofiles/boot/grub
	grub-mkrescue -o $(BOOT_IMG) $(BUILDDIR)/isofiles 2> /dev/null
	rm -r $(BUILDDIR)/isofiles

$(GRUB_CFG):
	@echo "set timeout=0" > $(GRUB_CFG)
	@echo "set default=0" >> $(GRUB_CFG)
	@echo "menuentry "BlightOS" {" >> $(GRUB_CFG)
	@echo "  multiboot2 /boot/kernel.elf" >> $(GRUB_CFG)
	@echo "  module2 /boot/shell.elf" >> $(GRUB_CFG)
	@echo "}" >> $(GRUB_CFG)

$(DISK_IMG):
	@echo "=============================================================="
	@echo "== Creating a test disk image with a GPT and a FAT32 Volume =="
	@echo "=============================================================="
	./mkdiskimg.sh

clean:
	make -C kernel clean
	rm -rf $(BUILDDIR)

run: $(BOOT_IMG) $(DISK_IMG)
	@echo "=============================================================="
	@echo "==             Running the QEMU x86_64 Emulator             =="
	@echo "=============================================================="
	qemu-system-x86_64 -enable-kvm -smp 4 -m 512M \
	-cpu host,migratable=no,+invtsc,+tsc-deadline \
	-monitor stdio -serial file:debug.log \
	-device ahci,id=ahci \
	-device ide-hd,drive=sata1,bus=ahci.1 \
	-device ide-hd,drive=sata2,bus=ahci.2 \
	-drive  id=sata1,file=$(BOOT_IMG),format=raw,if=none \
	-drive  id=sata2,file=$(DISK_IMG),format=raw,if=none \

run_noreboot: $(BOOT_IMG)
	@qemu-system-x86_64 -enable-kvm -smp 4 -m 512M \
	-cpu host,migratable=no,+invtsc,+tsc-deadline \
	-monitor stdio -serial file:debug.log \
	-drive file=$<,format=raw,media=disk \
	-no-reboot -no-shutdown

force: ;





include config.mk

BUILDDIR=build/
KERN_ELF=$(BUILDDIR)kernel.elf
KERN_BIN=$(BUILDDIR)kernel.bin
GRUB_CFG=$(BUILDDIR)grub.cfg
BOOT_IMG=$(BUILDDIR)boot.img
DISK_IMG=$(BUILDDIR)disk.img
USER_PROGS=	$(BUILDDIR)shell.box \
			$(BUILDDIR)test.box \
			$(BUILDDIR)fileman.box

all: kernel $(BOOT_IMG) $(DISK_IMG) run

kernel: force
	@echo "=============================================================="
	@echo "==                   Building the Kernel                    =="
	@echo "=============================================================="
	make -C kernel all

%.box: force
	@echo "=============================================================="
	@echo "==               Building User-space Program: $@"
	@echo "=============================================================="
	make -C $(patsubst $(BUILDDIR)%,programs/%,$(basename $@)) all

$(BOOT_IMG): $(KERN_ELF) $(GRUB_CFG)
	@echo "=============================================================="
	@echo "==  Createing a GRUB Resuce Disk Image to boot the Kernel   =="
	@echo "=============================================================="
	mkdir -p $(BUILDDIR)/isofiles/boot/grub
	cp $(KERN_ELF) $(BUILDDIR)/isofiles/boot/kernel.elf
	cp $(GRUB_CFG) $(BUILDDIR)/isofiles/boot/grub
	grub-mkrescue -o $(BOOT_IMG) $(BUILDDIR)/isofiles 2> /dev/null
	rm -r $(BUILDDIR)/isofiles

$(GRUB_CFG):
	@echo "set timeout=0" > $(GRUB_CFG)
	@echo "set default=0" >> $(GRUB_CFG)
	@echo "menuentry "BlightOS" {" >> $(GRUB_CFG)
	@echo "  multiboot2 /boot/kernel.elf" >> $(GRUB_CFG)
	@echo "}" >> $(GRUB_CFG)

$(DISK_IMG): $(KERN_ELF) $(USER_PROGS)
	@echo "=============================================================="
	@echo "== Creating a test disk image with a GPT and a FAT32 Volume =="
	@echo "=============================================================="
	./mkdiskimg.sh

clean:
	make -C kernel clean
	rm -rf $(BUILDDIR)

ifeq ($(ARCH), x86_64)
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
		-drive  id=sata2,file=$(DISK_IMG),format=raw,if=none

run_noreboot: $(BOOT_IMG)
	@qemu-system-x86_64 -enable-kvm -smp 4 -m 512M \
	-cpu host,migratable=no,+invtsc,+tsc-deadline \
	-monitor stdio -serial file:debug.log \
	-drive file=$<,format=raw,media=disk \
	-no-reboot -no-shutdown

else ifeq ($(ARCH), aarch64)
run: $(KERN_BIN) $(DISK_IMG)
	@echo "=============================================================="
	@echo "==            Running the QEMU AARCH64 Emulator             =="
	@echo "=============================================================="
	qemu-system-aarch64 -M raspi3 -smp 4 \
		-serial stdio \
		-drive file=$(DISK_IMG),format=raw,index=0,media=disk,if=sd \
		-kernel $(KERN_BIN) -dtb resources/bcm2710-rpi-3-b.dtb

# qemu-system-aarch64 \
#    -M raspi3b \
#    -cpu cortex-a53 \
#    -m 1G \
#    -kernel kernel8.img \
#    -dtb bcm2710-rpi-3-b-plus.dtb \
#    -drive "file=2023-05-03-raspios-bullseye-arm64-lite.img,format=raw,index=0,media=disk" \
#    -append "rw earlyprintk loglevel=8 console=ttyAMA0,115200 dwc_otg.lpm_enable=0 root=/dev/mmcblk0p2 rootdelay=1 systemd.run=/boot/firstrun.sh systemd.run_success_action=none debug systemd.unit=kernel-command-line.target" \
#    -usb \
#    -device usb-mouse \
#    -device usb-kbd \
#    -device usb-net,netdev=net0 \
#    -nographic \
#    -serial mon:stdio \
#    -netdev user,id=net0,hostfwd=tcp::7777-:22

endif

force: ;





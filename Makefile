BUILDDIR=build/
KERN_ELF=$(BUILDDIR)kernel.elf
GRUB_CFG=$(BUILDDIR)grub.cfg
BOOT_IMG=$(BUILDDIR)blightos.iso


all: clean kernel $(BOOT_IMG) run

kernel: force
	make -C kernel all

$(BOOT_IMG): $(KERN_ELF) $(GRUB_CFG)
	@mkdir -p $(BUILDDIR)/isofiles/boot/grub
	@cp $(KERN_ELF) $(BUILDDIR)/isofiles/boot/kernel.elf
	@cp $(GRUB_CFG) $(BUILDDIR)/isofiles/boot/grub
	@grub-mkrescue -o $(BOOT_IMG) $(BUILDDIR)/isofiles 2> /dev/null
	@rm -r $(BUILDDIR)/isofiles

$(GRUB_CFG):
	@echo "set timeout=0" > $(GRUB_CFG)
	@echo "set default=0" >> $(GRUB_CFG)
	@echo "menuentry "BlightOS" {" >> $(GRUB_CFG)
	@echo "  multiboot /boot/kernel.elf" >> $(GRUB_CFG)
	@echo "  boot" >> $(GRUB_CFG)
	@echo "}" >> $(GRUB_CFG)

clean:
	make -C kernel clean
	rm -f $(BOOT_IMG)

run: $(BOOT_IMG)
	@qemu-system-x86_64 -smp 4 -drive file=$<,format=raw,media=disk

force: ;


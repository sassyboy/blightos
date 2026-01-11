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
	@echo "  multiboot2 /boot/kernel.elf" >> $(GRUB_CFG)
	@echo "}" >> $(GRUB_CFG)

clean:
	make -C kernel clean
	rm -rf $(BOOT_IMG) $(BUILDDIR)*
	@mkdir -p $(BUILDDIR)/

run: $(BOOT_IMG)
	@qemu-system-x86_64 -enable-kvm -smp 4 -m 512M \
	-cpu host,migratable=no,+invtsc,+tsc-deadline \
	-monitor stdio -serial file:debug.log \
	-drive file=$<,format=raw,media=disk 

force: ;


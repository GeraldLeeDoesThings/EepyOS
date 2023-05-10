
TRIPLE=riscv64-none-elf
OBJCOPY:=$(TRIPLE)-objcopy

.PHONY: all kernel.img kernel.elf

all: kernel.img

clean:
	cargo clean
	rm -f kernel.img kernel.elf

kernel.img: kernel.elf
	$(OBJCOPY) kernel.elf -O binary kernel.img

kernel.elf:
	cargo build --release
	cp target/riscv64gc-unknown-none-elf/release/eepy_os ./kernel.elf


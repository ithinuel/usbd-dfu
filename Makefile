BOARD ?= duet3d
TARGET ?= thumbv7em-none-eabihf

.PHONY: all application bootloader

all: application bootloader

application bootloader:
	@cargo build --target ${TARGET} --release --features ${BOARD},$@,use-sha256 --example $@

#%.bin:
#    @cargo objcopy --target ${TARGET} --release --features ${BOARD},$(basename $@) --example $(basename $@) -- -O binary

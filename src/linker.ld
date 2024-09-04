ENTRY(_start)

MEMORY {
        ram (rwx) : ORIGIN = 0x40100000, LENGTH = 4095M
}

SECTIONS {

        PROVIDE(_ram_start = ORIGIN(ram));
        PROVIDE(_ram_length = LENGTH(ram));

        .text : {
                PROVIDE(_text_start = .);
                KEEP(*(.text.boot))
                *(.text .text.*)
                PROVIDE(_text_end = .);
        } >ram AT>ram :text

        PROVIDE(_global_pointer = .);

        .rodata : {
                PROVIDE(_rodata_start = .);
                *(.rodata .rodata.*)
                PROVIDE(_rodata_end = .);
        } >ram AT>ram :text

        .data : {
                PROVIDE(_data_start = .);
                *(.sdata .sdata.*)
                *(.data .data.*)
                PROVIDE(_data_end = .);
        } >ram AT>ram :data

        .bss (NOLOAD) : {
                PROVIDE(_bss_start = .);
                *(.sbss .sbss.*)
                *(.bss .bss.*)
                PROVIDE(_bss_end = .);
        } >ram AT>ram :bss

        .stack (NOLOAD) : ALIGN(16) {
                PROVIDE(_stack_top = .);
                . += 4K;
                PROVIDE(_stack_base = .);
        }

        .comment : {
                *(.comment .comment.*)
        }

        .riscv.attributes : {
                *(.riscv.attributes .riscv.attributes.*)
        }

        .symtab : ALIGN(8) {
                *(.symtab .symtab.*)
        }

        .shstrtab : {
                *(.shstrtab .shstrtab.*)
        }
        
        .strtab : {
                *(.strtab .strtab.*)
        }

        . = ALIGN(4K);
        PROVIDE(_heap_base = .);
}


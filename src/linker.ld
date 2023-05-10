ENTRY(_start)

MEMORY {
        ram (wxa) : ORIGIN = 0x82000000, LENGTH = 190M
}

SECTIONS {
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
                . = ALIGN(4096);
                PROVIDE(_data_start = .);
                *(.sdata .sdata.*)
                *(.data .data.*)
                PROVIDE(_data_end = .);
        } >ram AT>ram :data

        .bss : {
                PROVIDE(_bss_start = .);
                *(.sbss .sbss.*)
                *(.bss .bss.*)
                PROVIDE(_bss_end = .);
        } >ram AT>ram :bss

        PROVIDE(_ram_start = ORIGIN(ram));
        PROVIDE(_stack_start = _bss_end);
        PROVIDE(_stack_end = _stack_start + 0x40000);
        PROVIDE(_ram_end = _ram_start + LENGTH(ram));

        PROVIDE(_heap_start = _stack_end);
        PROVIDE(_heap_size = _ram_end - _heap_start);
        PROVIDE(_heap_end = _ram_end);

}


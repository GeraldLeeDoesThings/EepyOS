use core::arch::global_asm;

extern "C" {
    pub fn get_heap_base() -> *const u8;
}

global_asm!(include_str!("heap.S"));

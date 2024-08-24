use crate::println;

#[no_mangle]
extern "C" fn print_reg_hex(val: u64) {
    println!("{:#010x}", val);
}

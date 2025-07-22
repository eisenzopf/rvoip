use g729a_new::common::basic_operators::*;
use g729a_new::common::oper_32b::*;

#[test]
fn test_all_functions() {
    println!("rust_function_name,rust_output");
    println!("add,{}", add(10, 20));
    println!("sub,{}", sub(20, 10));
    println!("shl,{}", shl(10, 2));
    println!("shr,{}", shr(40, 2));
    println!("mult,{}", mult(16384, 16384));
    println!("L_mult,{}", l_mult(16384, 16384));
    println!("L_add,{}", l_add(10, 20));
    println!("L_sub,{}", l_sub(20, 10));
    println!("L_shl,{}", l_shl(10, 2));
    println!("L_shr,{}", l_shr(40, 2));
    println!("L_mac,{}", l_mac(10, 16384, 16384));
    println!("L_msu,{}", l_msu(10, 16384, 16384));
    println!("g729a_round,{}", round(536870912));
    let (hi, lo) = l_extract(536870912);
    println!("L_Extract,{},{}", hi, lo);
    println!("Mpy_32_16,{}", mpy_32_16(8192, 0, 2));
}

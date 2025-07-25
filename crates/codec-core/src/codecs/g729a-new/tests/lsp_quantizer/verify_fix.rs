// A simple test to verify the fix matches C output
use std::path::Path;
use std::env;

// Import the fixed module
#[path = "../../src/encoder/lsp_quantizer.rs"]
mod lsp_quantizer;
#[path = "../../src/encoder/lspvq.rs"]
mod lspvq;
#[path = "../../src/common/basic_operators.rs"]
mod basic_operators;
#[path = "../../src/common/tab_ld8a.rs"]
mod tab_ld8a;
#[path = "../../src/common/oper_32b.rs"]
mod oper_32b;

use lsp_quantizer::az_lsp;
use lspvq::LspQuantizer;
use basic_operators::Word16;

fn main() {
    let a = [4096, -4174, 1, 17, 12, 13, 13, 13, 11, 9, 103];
    let mut lsp = [0; 10];
    let mut lsp_q = [0; 10];
    let mut ana: [Word16; 2] = [0; 2];
    let old_lsp = [0; 10];

    println!("rust_function_name,rust_output");

    let mut quantizer = LspQuantizer::new();
    az_lsp(&a, &mut lsp, &old_lsp);
    quantizer.qua_lsp(&lsp, &mut lsp_q, &mut ana);

    for i in 0..10 {
        println!("lsp_q,{}", lsp_q[i]);
    }
    println!("ana0,{}", ana[0]);
    println!("ana1,{}", ana[1]);
} 
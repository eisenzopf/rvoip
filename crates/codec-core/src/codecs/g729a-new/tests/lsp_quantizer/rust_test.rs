
use g729a_new::encoder::lsp_quantizer::az_lsp;
use g729a_new::encoder::lspvq::{qua_lsp, lsp_encw_reset};

#[test]
fn test_lsp_quantizer() {
    let a = [4096, -4174, 1, 17, 12, 13, 13, 13, 11, 9, 103];
    let mut lsp = [0; 10];
    let mut lsp_q = [0; 10];
    let mut ana = [0; 2];
    let old_lsp = [0; 10];

    println!("rust_function_name,rust_output");

    lsp_encw_reset();
    az_lsp(&a, &mut lsp, &old_lsp);
    qua_lsp(&lsp, &mut lsp_q, &mut ana);

    for i in 0..10 {
        println!("lsp_q,{}", lsp_q[i]);
    }
    println!("ana0,{}", ana[0]);
    println!("ana1,{}", ana[1]);
}

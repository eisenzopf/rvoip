use g729a_new::encoder::lsp_quantizer::az_lsp;

#[test]
fn test_az_lsp() {
    let a = [4096, -4174, 1, 17, 12, 13, 13, 13, 11, 9, 103];
    let mut lsp = [0; 10];
    let old_lsp = [0; 10];

    println!("rust_function_name,rust_output");

    az_lsp(&a, &mut lsp, &old_lsp);

    for i in 0..10 {
        println!("az_lsp,{}", lsp[i]);
    }
}
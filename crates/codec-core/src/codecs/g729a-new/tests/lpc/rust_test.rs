use g729a_new::encoder::lpc::Lpc;

#[test]
fn test_lpc_functions() {
    let mut speech = [0i16; 240];
    let mut r_h = [0; 11];
    let mut r_l = [0; 11];
    let mut a = [0; 11];
    let mut rc = [0; 10];

    // Initialize speech with the same pattern as the C test
    for i in 0..240 {
        speech[i] = ((i * 100) % 10000) as i16;
    }

    let mut lpc = Lpc::new();

    println!("rust_function_name,rust_output");

    // Test autocorrelation
    lpc.autocorrelation(&speech, 10, &mut r_h, &mut r_l);
    for i in 0..=10 {
        println!("autocorr_rh,{}", r_h[i]);
    }
    for i in 0..=10 {
        println!("autocorr_rl,{}", r_l[i]);
    }

    // Test lag_window
    lpc.lag_window(10, &mut r_h, &mut r_l);
    for i in 0..=10 {
        println!("lag_window_rh,{}", r_h[i]);
    }
    for i in 0..=10 {
        println!("lag_window_rl,{}", r_l[i]);
    }

    // Test levinson
    lpc.levinson(&r_h, &r_l, &mut a, &mut rc);
    for i in 0..=10 {
        println!("levinson_a,{}", a[i]);
    }
    for i in 0..10 {
        println!("levinson_rc,{}", rc[i]);
    }
}

use g729a_new::encoder::pre_proc::PreProc;

#[test]
fn test_all_functions() {
    let mut signal = [8192; 80];
    let mut pre_proc = PreProc::new();
    pre_proc.process(&mut signal);

    println!("rust_function_name,rust_output");
    for sample in signal.iter() {
        println!("pre_proc,{}", sample);
    }
}

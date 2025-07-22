use g729a_new::common::basic_operators::mult;

#[test]
fn test_mult_isolated() {
    assert_eq!(mult(16384, 16384), 16384);
}

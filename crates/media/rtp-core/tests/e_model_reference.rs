use rvoip_rtp_core::quality::e_model::{
    evaluate, CodecImpairment, QualityInputs, G711, G722, G723_1_63K, G729, OPUS_24K,
};

const FIXTURE: &str = include_str!("fixtures/e_model_reference.csv");
const TOLERANCE: f32 = 0.01;

#[test]
fn matches_reference_vectors() {
    for line in FIXTURE
        .lines()
        .filter(|line| !line.starts_with('#'))
        .skip(1)
    {
        let columns: Vec<_> = line.split(',').collect();
        let actual = evaluate(QualityInputs {
            codec: codec(columns[0]),
            one_way_delay_ms: parse(columns[1]),
            loss_percent: parse(columns[2]),
        });

        assert_close(actual.r_factor, parse(columns[3]), line);
        assert_close(actual.mos_lq, parse(columns[4]), line);
        assert_close(actual.mos_cq, parse(columns[5]), line);
    }
}

fn codec(name: &str) -> CodecImpairment {
    match name {
        "g711" => G711,
        "g722_approx" => G722,
        "g729a_vad" => G729,
        "g723_1_63k_vad" => G723_1_63K,
        "opus_24k_approx" => OPUS_24K,
        _ => panic!("unknown fixture codec: {name}"),
    }
}

fn parse(value: &str) -> f32 {
    value.parse().expect("fixture values must be valid f32")
}

fn assert_close(actual: f32, expected: f32, case: &str) {
    assert!(
        (actual - expected).abs() <= TOLERANCE,
        "case {case}: expected {expected}, got {actual}"
    );
}

use rvoip_rtp_core::quality::e_model::{
    evaluate, CodecImpairment, QualityInputs, QualityScores, G711, G723_1_63K, G729, OPUS_24K,
    R_FACTOR_MAX,
};

fn scores(delay: f32, loss: f32, codec: CodecImpairment) -> QualityScores {
    evaluate(QualityInputs {
        one_way_delay_ms: delay,
        loss_percent: loss,
        codec,
    })
}

#[test]
fn r_factor_never_increases_with_loss() {
    let mut previous = R_FACTOR_MAX;
    for step in 0..=200 {
        let current = scores(25.0, step as f32 / 10.0, G711).r_factor;
        assert!(current <= previous);
        previous = current;
    }
}

#[test]
fn r_factor_never_increases_with_delay() {
    let mut previous = R_FACTOR_MAX;
    for delay in 0..=1000 {
        let current = scores(delay as f32, 1.0, G711).r_factor;
        assert!(current <= previous);
        previous = current;
    }
}

#[test]
fn ten_percent_g711_loss_does_not_zero_quality() {
    assert!(scores(25.0, 10.0, G711).r_factor > 0.0);
}

#[test]
fn delay_impairment_is_continuous_at_the_g107_knee() {
    const DELAY_KNEE_MS: f32 = 177.3;
    let before = scores(DELAY_KNEE_MS - 0.001, 0.0, G711).r_factor;
    let after = scores(DELAY_KNEE_MS + 0.001, 0.0, G711).r_factor;
    assert!((before - after).abs() < 0.001);
}

#[test]
fn scores_stay_within_model_limits() {
    let perfect = scores(0.0, 0.0, G711);
    let destroyed = scores(10_000.0, 100.0, G723_1_63K);
    assert!((4.3..=4.41).contains(&perfect.mos_lq));
    assert_eq!(destroyed.mos_cq, 1.0);
}

#[test]
fn listening_quality_is_never_below_conversational_quality() {
    for delay in (0..=1000).step_by(10) {
        let result = scores(delay as f32, 5.0, OPUS_24K);
        assert!(result.mos_lq >= result.mos_cq);
    }
}

#[test]
fn g711_never_scores_below_g729_for_equal_loss() {
    for loss in 0..=20 {
        let g711 = scores(25.0, loss as f32, G711).r_factor;
        let g729 = scores(25.0, loss as f32, G729).r_factor;
        assert!(g711 >= g729);
    }
}

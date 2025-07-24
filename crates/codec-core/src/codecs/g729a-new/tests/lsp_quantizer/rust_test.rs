use g729a_new::encoder::lsp_quantizer::{az_lsp, lsp_to_lsf, lsp_quantize, lsf_to_lsp};
use g729a_new::common::basic_operators::*;

#[test]
fn test_lsp_quantizer() {
    let mut lsp = [0; 10];
    let mut lsp_q = [0; 10];
    let mut ana = [0; 2];
    let a = [4096, -4174, 1, 17, 12, 13, 13, 13, 11, 9, 103];
    let old_lsp = [0; 10];

    az_lsp(&a, &mut lsp, &old_lsp);

    let mut freq_prev = [[0; 10]; 4];
    let freq_prev_reset = [
        2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396,
    ];
    for i in 0..4 {
        for j in 0..10 {
            freq_prev[i][j] = freq_prev_reset[j];
        }
    }

    let mut lsf = [0; 10];
    lsp_to_lsf(&lsp, &mut lsf);

    lsp_quantize(&lsf, &mut lsp_q, &mut ana, &mut freq_prev);

    let mut lsp_q_lsp = [0; 10];
    lsf_to_lsp(&lsp_q, &mut lsp_q_lsp);

    let expected_lsp_q = [
        32679, 31462, 26526, 19493, 10124, 143, -10137, -19210, -26520, -31159,
    ];
    assert_eq!(lsp_q, expected_lsp_q);
}
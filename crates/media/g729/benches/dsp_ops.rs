use criterion::{Criterion, black_box, criterion_group, criterion_main};

use g729::bitstream::{
    pack_sid_params, pack_speech_params, unpack_sid_params, unpack_speech_params,
};
use g729::dsp::arith::{add, mult, sub};
use g729::dsp::div::{div_s, inv_sqrt, log2, pow2};
use g729::dsp::oper32::{div_32, l_comp, l_extract, mpy_32};
use g729::dsp::{DspContext, Word16, Word32};

fn bench_arith(c: &mut Criterion) {
    c.bench_function("dsp/add_sub_mult", |b| {
        b.iter(|| {
            let mut ctx = DspContext::default();
            let mut acc = 0i16;
            for i in 0..256 {
                let a = add(&mut ctx, Word16(12345), Word16((2222 + i) as i16));
                let b = sub(&mut ctx, a, Word16((3333 - (i % 7)) as i16));
                acc ^= mult(&mut ctx, b, Word16(16384)).0;
            }
            black_box(acc);
        });
    });
}

fn bench_div_math(c: &mut Criterion) {
    c.bench_function("dsp/div_log_pow", |b| {
        b.iter(|| {
            let mut acc = 0i32;
            for i in 0..128 {
                let q = div_s(Word16(16384), Word16(32767 - (i % 31) as i16));
                let (e, f) = log2(Word32(0x3000_0000 - (i * 1024)));
                let p = pow2(e, f);
                acc ^= inv_sqrt(Word32(p.0 ^ i32::from(q.0))).0;
            }
            black_box(acc);
        });
    });
}

fn bench_oper32(c: &mut Criterion) {
    c.bench_function("dsp/oper32", |b| {
        b.iter(|| {
            let mut acc = 0i32;
            for i in 0..128 {
                let x = l_comp(Word16(0x4000), Word16((i * 17) as i16));
                let (hi, lo) = l_extract(x);
                let y = mpy_32(hi, lo, Word16(0x3000), Word16(0x2000));
                let (dhi, dlo) = l_extract(x);
                acc ^= div_32(black_box(y), dhi, dlo).0;
            }
            black_box(acc);
        });
    });
}

fn bench_bitstream(c: &mut Criterion) {
    c.bench_function("bitstream/pack_unpack", |b| {
        let speech = [
            Word16(0),
            Word16(120),
            Word16(210),
            Word16(1),
            Word16(6200),
            Word16(15),
            Word16(100),
            Word16(19),
            Word16(5300),
            Word16(8),
            Word16(110),
        ];
        let sid = [Word16(1), Word16(12), Word16(7), Word16(18)];
        b.iter(|| {
            let sbits = pack_speech_params(black_box(&speech));
            let _ = unpack_speech_params(black_box(&sbits));
            let sid_bits = pack_sid_params(black_box(&sid));
            let _ = unpack_sid_params(black_box(&sid_bits));
        });
    });
}

criterion_group!(
    benches,
    bench_arith,
    bench_div_math,
    bench_oper32,
    bench_bitstream
);
criterion_main!(benches);

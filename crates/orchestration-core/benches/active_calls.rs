use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::time::Duration;

#[path = "../tests/support/perf.rs"]
mod perf;

fn bench_active_call_lifecycle(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let mut group = c.benchmark_group("orchestration_active_call_lifecycle");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));

    for active_calls in perf::ACTIVE_CALL_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(active_calls),
            &active_calls,
            |bench, &active_calls| {
                bench.iter(|| {
                    let scenario = runtime
                        .block_on(perf::build_active_call_scenario(active_calls))
                        .expect("active call scenario");
                    black_box(scenario.call_ids.len());
                    black_box(scenario.offer_ids.len());
                    black_box(scenario);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_active_call_lifecycle);
criterion_main!(benches);

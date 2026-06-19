use agent_brain::bench::run_ci_bench;
use agent_brain::fixture::{new_isolated_engine, seed_bench_fixture, BENCH_FIXTURE_SKILLS};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn route_task_cache_hit(c: &mut Criterion) {
    let (engine, _dir) = new_isolated_engine().expect("isolated engine");
    seed_bench_fixture(&engine.store, BENCH_FIXTURE_SKILLS).expect("seed");
    let limits = agent_brain::fixture::default_route_limits();
    let query = "configure vitest for react testing";
    for i in 0..5 {
        let q = format!("{query} warmup {i}");
        engine
            .route_task(&q, None, &[], 500, limits, Some("implementing"), None)
            .expect("warmup");
    }
    c.bench_function("route_task_turn_cache_hit", |b| {
        b.iter(|| {
            engine
                .route_task(
                    black_box(query),
                    None,
                    &[],
                    500,
                    limits,
                    Some("implementing"),
            None,
        )
                .expect("route")
        });
    });
}

fn route_task_warm_unique(c: &mut Criterion) {
    let (engine, _dir) = new_isolated_engine().expect("isolated engine");
    seed_bench_fixture(&engine.store, BENCH_FIXTURE_SKILLS).expect("seed");
    let limits = agent_brain::fixture::default_route_limits();
    let mut i = 0usize;
    c.bench_function("route_task_warm_unique_query", |b| {
        b.iter(|| {
            let q = format!("implement rust module {i} with error handling patterns");
            i = i.wrapping_add(1);
            engine
                .route_task(black_box(&q), None, &[], 500, limits, Some("implementing"), None)
                .expect("route")
        });
    });
}

criterion_group!(benches, route_task_cache_hit, route_task_warm_unique);
criterion_main!(benches);

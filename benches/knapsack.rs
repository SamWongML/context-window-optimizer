use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ctx_optim::{
    selection::{
        diversity::DiversityConfig,
        knapsack::{greedy_knapsack, greedy_knapsack_diverse, kkt_knapsack, select_items},
    },
    types::{FileEntry, FileMetadata, ScoreSignals, ScoredEntry},
};
use std::{path::PathBuf, time::SystemTime};

fn make_scored(i: usize) -> ScoredEntry {
    ScoredEntry {
        entry: FileEntry {
            path: PathBuf::from(format!("src/dir{}/file_{i}.rs", i % 10)),
            token_count: 50 + (i % 300),
            hash: [i as u8; 16],
            metadata: FileMetadata {
                size_bytes: 200,
                last_modified: SystemTime::now(),
                git: None,
                language: None,
            },
            ast: None,
            simhash: None,
            content: None,
        },
        composite_score: (i % 100) as f32 / 100.0,
        signals: ScoreSignals {
            recency: (i % 10) as f32 / 10.0,
            size_score: 0.5,
            proximity: 0.0,
            dependency: 0.0,
        },
    }
}

fn bench_greedy_knapsack(c: &mut Criterion) {
    let mut group = c.benchmark_group("greedy_knapsack");
    let budget = 32_000;

    for n in [100, 1_000, 10_000] {
        let items: Vec<ScoredEntry> = (0..n).map(make_scored).collect();
        group.bench_with_input(BenchmarkId::from_parameter(n), &items, |b, items| {
            b.iter(|| greedy_knapsack(black_box(items.clone()), budget))
        });
    }
    group.finish();
}

fn bench_greedy_diverse(c: &mut Criterion) {
    let mut group = c.benchmark_group("greedy_knapsack_diverse");
    let budget = 32_000;
    let diversity = DiversityConfig::default();

    for n in [100, 1_000] {
        let items: Vec<ScoredEntry> = (0..n).map(make_scored).collect();
        group.bench_with_input(BenchmarkId::from_parameter(n), &items, |b, items| {
            b.iter(|| greedy_knapsack_diverse(black_box(items.clone()), budget, &diversity))
        });
    }
    group.finish();
}

fn bench_kkt_knapsack(c: &mut Criterion) {
    let mut group = c.benchmark_group("kkt_knapsack");
    let budget = 32_000;

    for n in [100, 1_000, 10_000] {
        let items: Vec<ScoredEntry> = (0..n).map(make_scored).collect();
        group.bench_with_input(BenchmarkId::from_parameter(n), &items, |b, items| {
            b.iter(|| kkt_knapsack(black_box(items.clone()), budget, None))
        });
    }
    group.finish();
}

fn bench_select_items_auto(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_items_auto");
    let budget = 32_000;

    for n in [100, 1_000] {
        let items: Vec<ScoredEntry> = (0..n).map(make_scored).collect();
        group.bench_with_input(BenchmarkId::from_parameter(n), &items, |b, items| {
            b.iter(|| select_items(black_box(items.clone()), budget, "auto", None))
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_greedy_knapsack,
    bench_greedy_diverse,
    bench_kkt_knapsack,
    bench_select_items_auto,
);
criterion_main!(benches);

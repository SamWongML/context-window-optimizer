use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ctx_optim::{
    selection::knapsack::greedy_knapsack,
    types::{FileEntry, FileMetadata, ScoreSignals, ScoredEntry},
};
use std::{path::PathBuf, time::SystemTime};

fn make_scored(i: usize) -> ScoredEntry {
    ScoredEntry {
        entry: FileEntry {
            path: PathBuf::from(format!("src/file_{i}.rs")),
            token_count: 50 + (i % 300),
            hash: [i as u8; 16],
            metadata: FileMetadata {
                size_bytes: 200,
                last_modified: SystemTime::now(),
                git: None,
                language: None,
            },
            ast: None,
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

fn bench_knapsack(c: &mut Criterion) {
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

criterion_group!(benches, bench_knapsack);
criterion_main!(benches);

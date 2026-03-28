use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ctx_optim::{
    config::ScoringWeights,
    scoring::{score_entries, score_entry},
    types::{FileEntry, FileMetadata, GitMetadata},
};
use std::{path::PathBuf, time::SystemTime};

fn make_entry(i: usize) -> FileEntry {
    FileEntry {
        path: PathBuf::from(format!("src/module_{i}.rs")),
        token_count: 100 + (i % 500),
        hash: [i as u8; 16],
        metadata: FileMetadata {
            size_bytes: 400 + i as u64,
            last_modified: SystemTime::now(),
            git: Some(GitMetadata {
                age_days: (i % 365) as f64,
                commit_count: (i % 100) as u32 + 1,
            }),
            language: Some(ctx_optim::types::Language::Rust),
        },
        ast: None,
    }
}

fn bench_score_single(c: &mut Criterion) {
    let entry = make_entry(0);
    let weights = ScoringWeights::default();
    c.bench_function("score_single_entry", |b| {
        b.iter(|| score_entry(black_box(&entry), &weights, &[], None))
    });
}

fn bench_score_batch(c: &mut Criterion) {
    let weights = ScoringWeights::default();
    let mut group = c.benchmark_group("score_batch");

    for n in [100, 1_000, 10_000] {
        let entries: Vec<FileEntry> = (0..n).map(make_entry).collect();
        group.bench_with_input(BenchmarkId::from_parameter(n), &entries, |b, entries| {
            b.iter(|| score_entries(black_box(entries), &weights, &[], None))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_score_single, bench_score_batch);
criterion_main!(benches);

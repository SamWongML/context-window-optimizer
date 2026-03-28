use criterion::{Criterion, criterion_group, criterion_main};
use ctx_optim::index::simhash::{find_near_duplicates, simhash_fingerprint};

fn bench_simhash_fingerprint(c: &mut Criterion) {
    let content_1k = vec![b'a'; 1_000];
    let content_10k = vec![b'a'; 10_000];

    let mut group = c.benchmark_group("simhash_fingerprint");
    group.bench_function("1KB", |b| {
        b.iter(|| simhash_fingerprint(&content_1k, 3));
    });
    group.bench_function("10KB", |b| {
        b.iter(|| simhash_fingerprint(&content_10k, 3));
    });
    group.finish();
}

fn bench_find_near_duplicates(c: &mut Criterion) {
    let fps_100: Vec<u64> = (0..100).map(|i| i * 7 + 42).collect();
    let fps_1k: Vec<u64> = (0..1_000).map(|i| i * 7 + 42).collect();

    let mut group = c.benchmark_group("find_near_duplicates");
    group.bench_function("100 items", |b| {
        b.iter(|| find_near_duplicates(&fps_100, 3, 4));
    });
    group.bench_function("1K items", |b| {
        b.iter(|| find_near_duplicates(&fps_1k, 3, 4));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_simhash_fingerprint,
    bench_find_near_duplicates
);
criterion_main!(benches);

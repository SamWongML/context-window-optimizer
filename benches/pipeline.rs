//! End-to-end pipeline benchmarks: discovery → dedup → score → pack.
//!
//! Measures the full `pack_files()` latency against the targets in CLAUDE.md:
//! - Score + pack in < 50ms
//! - Index 10K files in < 500ms

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use ctx_optim::{config::Config, pack_files, types::Budget};
use std::path::Path;
use tempfile::TempDir;

/// Build an in-memory temp repo with `n_files` small Rust source files.
///
/// Files are created once per benchmark group and reused across iterations
/// to avoid filesystem noise in the per-iteration measurements.
fn make_repo(n_files: usize) -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    for i in 0..n_files {
        let content = format!(
            "/// Module {i}.\npub fn function_{i}(x: usize) -> usize {{\n    x.wrapping_add({i})\n}}\n"
        );
        std::fs::write(src.join(format!("mod_{i}.rs")), content).unwrap();
    }
    dir
}

fn bench_pack_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("pack_pipeline");
    // Reduce sample count for larger inputs to keep total bench time reasonable.
    group.sample_size(20);

    let config = Config::default();
    let budget = Budget::standard(128_000);

    for n in [100usize, 1_000] {
        let repo = make_repo(n);
        group.bench_with_input(
            BenchmarkId::new("files", n),
            repo.path(),
            |b, path: &Path| {
                b.iter(|| {
                    pack_files(
                        black_box(path),
                        black_box(&budget),
                        black_box(&[]),
                        black_box(&config),
                    )
                    .expect("pack_files should not fail in benchmark")
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_pack_pipeline);
criterion_main!(benches);

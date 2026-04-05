//! Realistic scenario benchmarks verifying CLAUDE.md performance targets:
//! - Index 10K files in < 500ms
//! - Score + pack in < 50ms
//! - MCP tool response in < 100ms total

// The fixtures module is not available in bench crates (they compile separately).
// We inline the scenario builders here using a path include.
#[path = "../tests/fixtures/mod.rs"]
#[allow(dead_code, unused_imports)]
mod fixtures;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ctx_optim::{
    config::Config,
    index::discovery::{DiscoveryOptions, discover_files},
    pack_files,
    scoring::score_entries,
    selection::knapsack::select_items,
    types::Budget,
};

/// Benchmark: full pipeline on the scale_test(10_000) scenario.
/// Target: < 500ms for discovery; < 100ms total for score+pack on pre-discovered files.
fn bench_discover_10k(c: &mut Criterion) {
    let repo = fixtures::scenarios::scale_test(10_000);
    let config = Config::default();
    let opts = DiscoveryOptions::from_config(&config, repo.path());

    let mut group = c.benchmark_group("discover");
    group.sample_size(10); // expensive setup
    group.bench_function("10k_files", |b| {
        b.iter(|| discover_files(black_box(&opts)).expect("discover should not fail"));
    });
    group.finish();
}

/// Benchmark: scoring + knapsack on ~200 pre-discovered files.
/// Target: < 50ms.
fn bench_score_pack_200(c: &mut Criterion) {
    let repo = fixtures::scenarios::web_fullstack();
    let config = Config::default();
    let opts = DiscoveryOptions::from_config(&config, repo.path());
    let files = discover_files(&opts).expect("discover");
    let budget = Budget::standard(50_000);

    let mut group = c.benchmark_group("score_pack");
    group.bench_function("200_files", |b| {
        b.iter(|| {
            let scored = score_entries(
                black_box(&files),
                black_box(&config.weights),
                black_box(&[]),
                None,
            );
            select_items(
                scored,
                black_box(budget.l3_tokens()),
                black_box("auto"),
                None,
            )
        });
    });
    group.finish();
}

/// Benchmark: full pipeline on polyglot_monorepo (~400 files).
/// Target: < 100ms.
fn bench_full_pipeline_medium(c: &mut Criterion) {
    let repo = fixtures::scenarios::polyglot_monorepo();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);
    group.bench_function("400_files_polyglot", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config),
            )
            .expect("pack should not fail")
        });
    });
    group.finish();
}

/// Benchmark: full pipeline on scale_test(5000).
fn bench_full_pipeline_large(c: &mut Criterion) {
    let repo = fixtures::scenarios::scale_test(5_000);
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);
    group.bench_function("5k_files_scale", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config),
            )
            .expect("pack should not fail")
        });
    });
    group.finish();
}

/// Benchmark: dedup overhead on legacy_with_duplication.
fn bench_dedup_heavy(c: &mut Criterion) {
    let repo = fixtures::scenarios::legacy_with_duplication();

    let mut group = c.benchmark_group("dedup");
    group.sample_size(10);

    // Exact dedup only
    let config_exact = Config::default();
    let budget = Budget::standard(128_000);
    group.bench_function("exact_only", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config_exact),
            )
            .expect("pack should not fail")
        });
    });

    // Exact + near dedup
    let mut config_near = Config::default();
    config_near.dedup.near = true;
    config_near.dedup.hamming_threshold = 5;
    group.bench_function("exact_plus_near", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config_near),
            )
            .expect("pack should not fail")
        });
    });

    group.finish();
}

/// Benchmark: focus-path scoring vs no focus.
fn bench_focus_vs_no_focus(c: &mut Criterion) {
    let repo = fixtures::scenarios::rust_workspace();
    let config = Config::default();
    let budget = Budget::standard(50_000);

    let mut group = c.benchmark_group("focus");
    group.sample_size(10);

    group.bench_function("no_focus", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config),
            )
            .expect("pack")
        });
    });

    let focus = vec![repo.path().join("server/src/module_0.rs")];
    group.bench_function("with_focus", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&focus),
                black_box(&config),
            )
            .expect("pack")
        });
    });

    group.finish();
}

/// Benchmark: full pipeline on scale_test_medium(5000).
/// P3 target: significant improvement over scale_test(5000) due to tight budget.
fn bench_full_pipeline_medium_files(c: &mut Criterion) {
    let repo = fixtures::scenarios::scale_test_medium(5_000);
    let config = Config::default();
    let budget = Budget::standard(128_000);

    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);
    group.bench_function("5k_medium_files", |b| {
        b.iter(|| {
            pack_files(
                black_box(repo.path()),
                black_box(&budget),
                black_box(&[]),
                black_box(&config),
            )
            .expect("pack should not fail")
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_discover_10k,
    bench_score_pack_200,
    bench_full_pipeline_medium,
    bench_full_pipeline_large,
    bench_full_pipeline_medium_files,
    bench_dedup_heavy,
    bench_focus_vs_no_focus,
);
criterion_main!(benches);

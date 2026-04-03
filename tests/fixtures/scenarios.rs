//! Named scenario constructors for integration and benchmark tests.
//!
//! Each function composes [`RealisticRepoBuilder`] to produce a deterministic,
//! self-contained [`RealisticRepo`] that exercises a distinct structural pattern:
//! web full-stack, Rust workspace, polyglot monorepo, legacy codebase with heavy
//! duplication, and an arbitrary-scale benchmark target.

use super::builder::{CommitPattern, Lang, RealisticRepo, RealisticRepoBuilder};
use super::content::FileSize;

// ── 1. Web full-stack (~200 files) ────────────────────────────────────────────

/// Build a web full-stack repository (~200 files).
///
/// Structure:
/// - Frontend: 40 TS components, 10 TS hooks, 5 TS style utils
/// - Backend: 25 TS API routes, 8 TS middleware, 15 TS models, 10 TS utils
/// - Tests: 30 TS test files
/// - Non-code files
/// - Vendor exact duplicates: 3 groups × 5 files (15 files, 12 dedup)
/// - Git: initial commit 180 days ago, Hotspot on components/ + api/routes/,
///   150 commits over 180 days
///
/// # Examples
///
/// ```no_run
/// use tests::fixtures::scenarios::web_fullstack;
/// let repo = web_fullstack();
/// assert!(repo.path().join("src/components").exists());
/// ```
pub fn web_fullstack() -> RealisticRepo {
    // Build hot-path lists before consuming the builder.
    let hot_paths: Vec<String> = (0..40)
        .map(|i| format!("src/components/module{i}.ts"))
        .chain((0..25).map(|i| format!("src/api/routes/module{i}.ts")))
        .collect();

    RealisticRepoBuilder::new()
        .seed(1)
        // Frontend
        .add_module("src/components", Lang::TypeScript, 40)
        .add_module("src/hooks", Lang::TypeScript, 10)
        .add_module("src/styles", Lang::TypeScript, 5)
        // Backend
        .add_module("src/api/routes", Lang::TypeScript, 25)
        .add_module("src/api/middleware", Lang::TypeScript, 8)
        .add_module("src/models", Lang::TypeScript, 15)
        .add_module("src/utils", Lang::TypeScript, 10)
        // Tests
        .add_module("tests", Lang::TypeScript, 30)
        // Non-code
        .add_non_code_files()
        // Vendor exact duplicates: 3 groups of 5
        .add_exact_duplicates("vendor/group_a", 5)
        .add_exact_duplicates("vendor/group_b", 5)
        .add_exact_duplicates("vendor/group_c", 5)
        // Git history
        .initial_commit(180)
        .commit_pattern(CommitPattern::Hotspot {
            hot_paths,
            total_commits: 150,
            span_days: 180,
        })
        .build()
}

// ── 2. Rust workspace (~150 files) ────────────────────────────────────────────

/// Build a Rust workspace repository (~150 files).
///
/// Structure:
/// - 4 crates: core (25 src + 10 tests), server (20 src + 10 handlers + 8 tests),
///   cli (10 src), macros (5 src)
/// - Each crate gets a Cargo.toml; workspace root Cargo.toml
/// - 5 bench files, non-code files
/// - Git: initial commit 90 days ago, Uniform 100 commits over 90 days,
///   recent_edits on server/src/ files at 1 day ago
///
/// # Examples
///
/// ```no_run
/// use tests::fixtures::scenarios::rust_workspace;
/// let repo = rust_workspace();
/// assert!(repo.path().join("core/src").exists());
/// ```
pub fn rust_workspace() -> RealisticRepo {
    // Collect server/src/ paths for recent_edits.
    let server_src_paths: Vec<String> =
        (0..20).map(|i| format!("server/src/module_{i}.rs")).collect();

    RealisticRepoBuilder::new()
        .seed(2)
        // crate: core
        .add_module("core/src", Lang::Rust, 25)
        .add_module("core/tests", Lang::Rust, 10)
        .add_file_with_content(
            "core/Cargo.toml",
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        // crate: server
        .add_module("server/src", Lang::Rust, 20)
        .add_module("server/src/handlers", Lang::Rust, 10)
        .add_module("server/tests", Lang::Rust, 8)
        .add_file_with_content(
            "server/Cargo.toml",
            "[package]\nname = \"server\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        // crate: cli
        .add_module("cli/src", Lang::Rust, 10)
        .add_file_with_content(
            "cli/Cargo.toml",
            "[package]\nname = \"cli\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        // crate: macros
        .add_module("macros/src", Lang::Rust, 5)
        .add_file_with_content(
            "macros/Cargo.toml",
            "[package]\nname = \"macros\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        // Workspace root
        .add_file_with_content(
            "Cargo.toml",
            "[workspace]\nmembers = [\"core\", \"server\", \"cli\", \"macros\"]\nresolver = \"2\"\n",
        )
        // Benches
        .add_module("benches", Lang::Rust, 5)
        // Non-code
        .add_non_code_files()
        // Git history
        .initial_commit(90)
        .commit_pattern(CommitPattern::Uniform {
            total_commits: 100,
            span_days: 90,
        })
        .recent_edits(server_src_paths, 1)
        .build()
}

// ── 3. Polyglot monorepo (~400 files) ─────────────────────────────────────────

/// Build a polyglot monorepo (~400 files).
///
/// Structure:
/// - Go API service (50 files) + Dockerfile + Makefile
/// - Python ML service (60 files) + Dockerfile + Makefile
/// - TS web service (80 files) + Dockerfile + Makefile
/// - Shared Go lib (30 files)
/// - Infrastructure YAML, non-code files
/// - 10 near-duplicates in services/config/ at 0.90 similarity
/// - 5 Python doc scripts, 15 Python scripts
/// - Git: initial commit 180 days ago, Stale with burst on services/web/ at 3 days
///
/// # Examples
///
/// ```no_run
/// use tests::fixtures::scenarios::polyglot_monorepo;
/// let repo = polyglot_monorepo();
/// assert!(repo.path().join("services/api").exists());
/// ```
pub fn polyglot_monorepo() -> RealisticRepo {
    // Burst paths: some files in services/web/ (TS module paths)
    let burst_paths: Vec<String> =
        (0..10).map(|i| format!("services/web/module{i}.ts")).collect();

    RealisticRepoBuilder::new()
        .seed(3)
        // Go API service
        .add_module("services/api", Lang::Go, 50)
        .add_file_with_content(
            "services/api/Dockerfile",
            "FROM golang:1.22-alpine\nWORKDIR /app\nCOPY . .\nRUN go build -o api .\nCMD [\"./api\"]\n",
        )
        .add_file_with_content(
            "services/api/Makefile",
            "build:\n\tgo build -o api .\ntest:\n\tgo test ./...\n",
        )
        // Python ML service
        .add_module("services/ml", Lang::Python, 60)
        .add_file_with_content(
            "services/ml/Dockerfile",
            "FROM python:3.12-slim\nWORKDIR /app\nCOPY requirements.txt .\nRUN pip install -r requirements.txt\nCOPY . .\nCMD [\"python\", \"main.py\"]\n",
        )
        .add_file_with_content(
            "services/ml/Makefile",
            "install:\n\tpip install -r requirements.txt\ntest:\n\tpytest\n",
        )
        // TS web service
        .add_module("services/web", Lang::TypeScript, 80)
        .add_file_with_content(
            "services/web/Dockerfile",
            "FROM node:20-alpine\nWORKDIR /app\nCOPY package*.json .\nRUN npm ci\nCOPY . .\nRUN npm run build\nCMD [\"npm\", \"start\"]\n",
        )
        .add_file_with_content(
            "services/web/Makefile",
            "install:\n\tnpm ci\nbuild:\n\tnpm run build\ntest:\n\tnpm test\n",
        )
        // Shared Go lib
        .add_module("libs/shared", Lang::Go, 30)
        // Infrastructure YAML
        .add_file_with_content(
            "infra/k8s/deployment.yaml",
            "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\nspec:\n  replicas: 2\n",
        )
        .add_non_code_files()
        // Near-duplicates in services/config/
        .add_near_duplicates("services/config", 10, 0.90)
        // Python doc scripts and utility scripts
        .add_module("scripts/docs", Lang::Python, 5)
        .add_module("scripts/utils", Lang::Python, 15)
        // Git history
        .initial_commit(180)
        .commit_pattern(CommitPattern::Stale {
            initial_age_days: 180,
            burst_paths,
            burst_age_days: 3,
        })
        .build()
}

// ── 4. Legacy with duplication (~200 files) ───────────────────────────────────

/// Build a legacy codebase repository with heavy duplication (~200 files).
///
/// Structure:
/// - 25 near-duplicate handlers at 0.95 similarity + 5 unique handler files
/// - 20 model files, 15 utility files
/// - 4 groups × 5 exact duplicates in vendor/
/// - Core: 7 regular files + 3 large files (7000, 5000, 6000 tokens)
/// - 20 test files, non-code files
/// - Git: initial commit 365 days ago, Stale with burst on src/core/ at 3 days
///
/// # Examples
///
/// ```no_run
/// use tests::fixtures::scenarios::legacy_with_duplication;
/// let repo = legacy_with_duplication();
/// assert!(repo.path().join("src/handlers").exists());
/// ```
pub fn legacy_with_duplication() -> RealisticRepo {
    // Burst on src/core/ regular files
    let burst_paths: Vec<String> =
        (0..7).map(|i| format!("src/core/module_{i}.rs")).collect();

    RealisticRepoBuilder::new()
        .seed(4)
        // Near-duplicate handlers
        .add_near_duplicates("src/handlers", 25, 0.95)
        // Unique handler files
        .add_module("src/handlers", Lang::Rust, 5)
        // Models and utils
        .add_module("src/models", Lang::Rust, 20)
        .add_module("src/utils", Lang::Rust, 15)
        // Vendor exact duplicates: 4 groups of 5
        .add_exact_duplicates("vendor/pkg_a", 5)
        .add_exact_duplicates("vendor/pkg_b", 5)
        .add_exact_duplicates("vendor/pkg_c", 5)
        .add_exact_duplicates("vendor/pkg_d", 5)
        // Core: 7 regular files
        .add_module("src/core", Lang::Rust, 7)
        // Core: 3 large files
        .add_large_file("src/core/large_a.rs", 7000)
        .add_large_file("src/core/large_b.rs", 5000)
        .add_large_file("src/core/large_c.rs", 6000)
        // Tests
        .add_module("tests", Lang::Rust, 20)
        // Non-code
        .add_non_code_files()
        // Git history
        .initial_commit(365)
        .commit_pattern(CommitPattern::Stale {
            initial_age_days: 365,
            burst_paths,
            burst_age_days: 3,
        })
        .build()
}

// ── 5. Scale test (n files) ───────────────────────────────────────────────────

/// Build a scale-test repository with exactly `n` files.
///
/// Files are spread across 10 subdirectories (`src/mod_0/` through `src/mod_9/`).
/// Every 5th file uses [`FileSize::Small`]; the rest use [`FileSize::Tiny`].
/// A single initial commit is placed 30 days ago (minimal git for benchmark speed).
///
/// # Examples
///
/// ```no_run
/// use tests::fixtures::scenarios::scale_test;
/// let repo = scale_test(1000);
/// assert!(repo.path().join("src/mod_0").exists());
/// ```
pub fn scale_test(n: usize) -> RealisticRepo {
    let mut builder = RealisticRepoBuilder::new().seed(5).initial_commit(30);

    for i in 0..n {
        let subdir = i % 10;
        let size = if i % 5 == 0 {
            FileSize::Small
        } else {
            FileSize::Tiny
        };
        let path = format!("src/mod_{subdir}/file_{i}.rs");
        builder = builder.add_file(path, Lang::Rust, size);
    }

    builder.build()
}

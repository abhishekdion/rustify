# Rustify — Project Plan (Open-Source Universal Python→Rust Transpiler)

## 1. Vision
A **universal, production-grade Python → Rust transpiler** that converts any valid
CPython program into idiomatic, safe, fast Rust — with a desktop GUI, a CLI, and
a library API. Target: open-source, community-driven, company-grade engineering.

## 2. Goals & Non-Goals
### Goals
- Support the full CPython language (syntax + stdlib subset with shims).
- Type inference + gradual typing (Python type hints → Rust types; fallback to
  dynamic `PyObject` shim for untyped code).
- Incremental transpilation (only changed modules).
- Cross-platform GUI (Windows primary, macOS/Linux).
- Plugin system for custom builtins/modules.
- Resource-friendly: low memory, fast startup, smooth UI (60fps).

### Non-Goals (v1)
- Bytecode-level compatibility; C-extensions (use PyO3 interop instead).
- Perfect performance parity for dynamic code (shim overhead accepted).

## 3. Tech Stack (resource-friendly)
| Layer | Choice | Rationale |
|---|---|---|
| Core transpiler | Rust (stable) | Fast, safe, no GC |
| Parser | `rustpython-parser` or custom (reuse existing) | CPython grammar fidelity |
| Type inference | Hindley-Milner + Python hints | Sound + practical |
| GUI | **Tauri 2** + **React/TypeScript** (or egui) | Small binary, web tech, ~5MB; native feel |
| Async runtime | Tokio (only where needed) | Optional |
| DB/cache | SQLite (rusqlite) for project index | Lightweight |
| Packaging | cargo-dist + Tauri bundler | MSI/NSIS on Windows |
| CI | GitHub Actions | Standard |
| Lint/format | rustfmt, clippy, ruff (Python tests) | Quality gates |

**Why not Electron**: Tauri produces ~5MB binaries vs ~150MB Electron, lower RAM.

## 4. Repository Layout (monorepo)
```
Rustify/
├── crates/
│   ├── rustify-core/          # lexer, parser, AST, types, infer, codegen
│   ├── rustify-runtime/       # prelude shim crate (published)
│   ├── rustify-cli/           # CLI binary
│   └── rustify-lsp/           # (later) language server
├── apps/
│   └── desktop/               # Tauri + React GUI
├── plugins/                   # example plugins
├── tests/
│   ├── corpus/                # Python test corpus (thousands of files)
│   ├── golden/                # expected Rust output
│   └── e2e/
├── benches/
├── docs/
│   ├── architecture.md
│   ├── contributing.md
│   └── user-guide.md
├── .github/
│   ├── workflows/             # ci.yml, release.yml
│   ├── ISSUE_TEMPLATE/
│   └── PULL_REQUEST_TEMPLATE.md
├── Cargo.toml                 # workspace
├── README.md
├── LICENSE (MIT OR Apache-2.0)
├── CONTRIBUTING.md
├── CODE_OF_CONDUCT.md
└── SECURITY.md
```

## 5. Branching Strategy (company-grade)
- **main**: always releasable, protected, requires PR + CI green + 1 review.
- **develop**: integration branch for next release.
- **feature/<ticket>-short-desc**: feature work, PR → develop.
- **fix/<ticket>-short-desc**: bug fixes.
- **release/x.y.z**: release prep (version bump, changelog); merges to main + develop.
- **hotfix/x.y.z**: urgent production fix from main.
- Tags: `vX.Y.Z` (semver). Releases via GitHub Releases + cargo-dist.

Commit convention: Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
PRs: linked issue, tests, changelog entry (changesets).

## 6. UI / UX (Windows desktop)
- Layout: sidebar project tree, center code editor (Monaco) with Python/Rust split view,
  bottom panel (diagnostics, terminal, benchmarks).
- Features: drag-drop project, transpile button, live diagnostics, diff view,
  settings (target Rust edition, optimization level), progress + cancel.
- Resource-friendly: virtualized lists, lazy file loading, background transpile
  thread, debounced UI updates, no polling.

## 7. Milestones
- **M0 (now)**: rename done, folder organized. ✅
- **M1**: Workspace refactor into crates; CLI preserved.
- **M2**: Full grammar coverage + dynamic shim (`PyObject`).
- **M3**: Tauri desktop app MVP.
- **M4**: Incremental builds + project index (SQLite).
- **M5**: Plugin API + docs + 1.0 release.
- **M6**: LSP + IDE extensions.

## 8. Quality Gates
- Unit + integration tests (cargo test), golden-file tests.
- Property tests (proptest) for lexer/parser.
- Benchmarks (criterion) tracked in CI.
- Coverage ≥ 80% on core.
- MSRV pinned; Dependabot for deps.

## 9. Open-Source Readiness
- MIT/Apache-2.0 dual license, DCO sign-off, Code of Conduct, security policy.
- Issue/PR templates, labels, milestones.
- Docs site (mdBook) published via GitHub Pages.
- Discord/forum for community.

## 10. Immediate Next Steps
1. Convert current single-crate into Cargo workspace (M1).
2. Add GitHub repo + branch protection + CI.
3. Scaffold Tauri app in apps/desktop.
4. Write CONTRIBUTING.md and issue templates.
5. Set up changesets for versioning.

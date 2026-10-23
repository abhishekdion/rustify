# Contributing to Rustify

Thanks for your interest in contributing!

## Getting started

```powershell
git clone https://github.com/<your-org>/rustify.git
cd rustify
cargo build --workspace
cargo test --workspace
```

## Branching model

| Branch | Purpose |
|---|---|
| `main` | Always releasable. Protected: PR + green CI + 1 review required. |
| `develop` | Integration for the next release. |
| `feature/<ticket>-short-desc` | New work, PR into `develop`. |
| `fix/<ticket>-short-desc` | Bug fixes, PR into `develop`. |
| `release/x.y.z` | Release prep (version bump + changelog); merges to `main` and `develop`. |
| `hotfix/x.y.z` | Urgent fixes from `main`, merged back into both. |

Releases are tagged `vX.Y.Z` (SemVer) on `main`.

## Commits & PRs

- Conventional Commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- Every PR: linked issue, tests for changed behaviour, green CI.
- Keep PRs small and focused; one logical change per PR.

## Quality gates

- `cargo fmt --check` and `cargo clippy -- -D warnings` must pass.
- New pipeline features need unit tests (and a golden-file test where relevant).
- Anything the transpiler cannot convert must surface as a **diagnostic**, never a
  silent mistranslation.

## Reporting issues

Use the issue templates. For security issues see SECURITY.md.

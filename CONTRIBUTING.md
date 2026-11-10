Default branch: `main`. Keep the history readable: Conventional Commits
(`feat:`, `fix:`, `docs:`, `test:`, `chore:`, ...), see the
[specification](https://www.conventionalcommits.org/en/v1.0.0/).

## Checks before opening a PR

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

## Project rules that matter

- No `panic!()` / `unwrap()` in library code; fallible constructors return `Result`.
- Dependencies use exact pins (`tracing = "=0.1.41"`) and must build on windows, linux, macos.
- No section-bar comment banners in Rust sources; comments only where code is complex.
- Changes that affect users get an entry under `## [Unreleased]` in `CHANGELOG.md`
  (Keep a Changelog notation: Added, Changed, Deprecated, Removed, Fixed, Security).

## AI assistance

This project is developed with AI agents (see the README AI disclosure). If you used an AI
assistant in your PR, say so in the PR description and be ready to explain the code.

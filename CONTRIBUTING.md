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

## Release routine (maintainers)

Run on `main` with a clean tree (`git status --short` prints nothing). The steps mirror
`.github/workflows/ci.yml` and `.github/workflows/release.yml`; the workflows themselves only run
on GitHub, so run their local equivalents before pushing.

1. Local CI gate:

   ```sh
   cargo fmt --all --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-targets
   ```

2. Pick the version per [`docs/semantic-versioning.md`](docs/semantic-versioning.md): in `0.y.z`
   the minor version goes up for features, the patch version for fixes.

3. Regenerate the changelog from history, naming the tag that is about to be created:

   ```sh
   git-cliff --tag v0.1.1 -o CHANGELOG.md
   ```

   Then hand-check the result: `## [Unreleased]` first, sections in Added / Changed / Deprecated /
   Removed / Fixed / Security order, every `## [x.y.z]` heading has a link ref at the bottom, and
   the header marker `<!-- git-cliff: end of header -->` appears exactly once. Full regeneration
   (`git-cliff -o CHANGELOG.md`, `git-cliff --tag ...`) is the supported mode: `--prepend` drops the
   reference links.

4. Bump the version and refresh the lock file:

   ```sh
   cargo set-version 0.1.1                                # cargo-edit, or edit Cargo.toml
   cargo metadata --offline --format-version 1 > /dev/null # sh; refreshes the Cargo.lock version entry
   ```

   In PowerShell replace `> /dev/null` with `| Out-Null`.

5. Commit the release, tag it, push. Pushing the tag starts the release workflow:

   ```sh
   git add Cargo.toml Cargo.lock CHANGELOG.md
   git commit -m "chore(release): prepare for v0.1.1"
   git tag -a v0.1.1 -m "v0.1.1"
   git push origin main --follow-tags
   ```

   `chore(release): prepare for` commits are skipped by `cliff.toml`, so the release commit never
   shows up in the notes.

6. Local release binaries (optional; `dist/` is gitignored, CI builds the matrix):

   ```powershell
   scripts/build-release.ps1        # windows, static CRT
   scripts/build-release.sh --musl  # linux musl, fully static
   ```

   Output: `dist/silv-<version>-<target>[.exe]` plus `dist/SHA256SUMS`.

7. Verify what GitHub published: three binaries plus `SHA256SUMS`, real release notes, then
   `sha256sum -c SHA256SUMS` against the downloaded assets and a smoke run of one binary.

8. To republish notes or assets without moving the tag: Actions -> `release` -> Run workflow, and
   pass the tag. Never move a tag that was pushed: consumers pin on it.

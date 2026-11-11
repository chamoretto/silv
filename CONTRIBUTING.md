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

## Local CI gate

CI runs fmt, clippy, build and test on Linux, Windows and macOS. Parts of the code are platform gated
(the xkb keymap compiles only on unix, the macOS keymap only on macOS), so checking on one platform is
not enough. Run the same gates locally before pushing:

```powershell
pwsh -File scripts/ci-local.ps1                        # all gates
pwsh -File scripts/ci-local.ps1 -Quick                 # lib tests only under WSL (/mnt/c builds are slow)
pwsh -File scripts/ci-local.ps1 -SkipWsl -SkipMacos    # the Windows gate only
```

The WSL gate needs a Rust toolchain in the distro (`SILV_WSL_DISTRO` overrides the default
`Ubuntu-24.04`). The macOS gate is a cross compile: checks and clippy only, its tests have to run on a
Mac, which CI does.

### macOS tests run single threaded

`cargo test` on macOS needs `--test-threads=1`:

```sh
cargo test --all-targets -- --test-threads=1
```

The macOS key map probe reads the Carbon text input source API, which answers only on the thread that
owns the GUI session. The test harness runs tests on parallel worker threads; calling that API from
one aborts the whole test binary with SIGABRT, with no Rust panic and no reference to the probe in
the output. The probe guards itself with `pthread_main_np()` and degrades to the built-in table off
the main thread, so a multi-threaded run survives and reports the probe test as skipped, and a
single threaded run executes it. CI passes the flag on the macOS job.

Prefer feature branches with pull requests: `ci.yml` runs the identical gate on `pull_request`, so
`main` stays releasable and a platform-gated break is caught before it lands.

## Changelog automation (git-cliff)

The `Unreleased` section of `CHANGELOG.md` is kept in step with history automatically:
`.githooks/post-commit` runs `git-cliff` after every commit and folds the refreshed file into
that commit by amending. One-time setup per clone:

```sh
git config core.hooksPath .githooks
```

The hook skips rebase, cherry-pick and merge sequences, never re-enters its own amend
(guarded by `SILV_CHANGELOG_HOOK`), and does nothing on machines without `git-cliff`
(`cargo install --locked git-cliff`, `winget`, or GitHub release binaries). The changelog is
commit-based: each entry is one commit's subject line, and released sections link the commit
SHA; `Unreleased` entries stay plain because their SHAs are not final yet.

## Release routine (maintainers)

Run on `main` with a clean tree (`git status --short` prints nothing). The steps mirror
`.github/workflows/ci.yml` and `.github/workflows/release.yml`; the workflows themselves only run
on GitHub, so run their local equivalents before pushing.

1. Pick the version per [`docs/semantic-versioning.md`](docs/semantic-versioning.md): in `0.y.z`
   the minor version goes up for features, the patch version for fixes.

2. Bump the version:

   ```sh
   cargo set-version 0.1.1     # cargo-edit (cargo install --locked cargo-edit), or edit Cargo.toml
   ```

3. Regenerate the changelog from history, naming the tag that is about to be created (the
   post-commit hook keeps `Unreleased` current, so this step turns it into the release
   section and attaches the commit SHA links):

   ```sh
   git-cliff --tag v0.1.1 -o CHANGELOG.md
   ```

   Then hand-check the result: `## [Unreleased]` first (the heading comes from the body template, the header ends at the marker), sections in Added / Changed / Deprecated /
   Removed / Fixed / Security order, every `## [x.y.z]` heading has a link ref at the bottom (the `Unreleased` link points at the latest tag), and
   the header marker `<!-- git-cliff: end of header -->` appears exactly once. Full regeneration
   (`git-cliff -o CHANGELOG.md`, `git-cliff --tag ...`) is the supported mode: `--prepend` drops the
   reference links.

4. Local CI gate, after the bump: the build refreshes the `Cargo.lock` entry for the package, so
   `git diff -- Cargo.lock` should end up showing only that version line.

   ```sh
   cargo fmt --all --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-targets
   ```

   The `ci` workflow repeats the clippy/test legs on ubuntu, windows and macos; a local run on one
   platform is not enough before a release, so push the branch and watch the workflow too.

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

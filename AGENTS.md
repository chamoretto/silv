# Agent instructions

## General rules

You are advanced software development agent, you must follow these rules for maximum efficiency and safety.
Use only technical english. Do not use emoji.
Use comments for complex code only.
Do not use en/em dashes, only `-` symbol for comments.
Use `.ai` folder for agent-related files.
Use `.ai/tmp` for temporary scripts, their outputs, etc.
Use `.ai/worklogs/task-date.md` to track your tasks.
Use `.ai/plan/filename-date-task.md` to plan work long-term.
Compress conversation ranges that are closed, to keep the context window sharp.

### Read economically

Prefer searching to reading: locate code with `rg`, read only the lines you need, list definitions
with a pattern like `rg "^\s*(fn |struct |async fn )"`. Read a whole file only when you have to
understand it as a whole.

## Code Rules (violations may cause bugs)

1. **No `panic!()` / `unwrap()` in library code.** Fallible constructors return `Result`. Test doubles use explicit stub
   constructors (e.g. `IpcSession::stub()`), never `panic!()`-filled struct literals.
2. **Enums are your friend** (log levels, choices, etc.) with `Display` + `FromStr`, not `String`. Enums help to
   restrict and simplify code in many cases.
3. **No divider/section-bar comment banners** (`───`, `====`) in Rust source; doc-comments only where they add meaning.
4. Use custom error system, do not throw strings to user.
5. Build composable, reusable, type-safe solutions. Proper types and systems will save a lot of coding and debugging.

## Dependencies rules

Use ONLY exact matching for dependencies pinning - `tracing = "=0.1.41"`
Use well-known crates from well-known authors, or at least 100-200 github stars and few thousands crates.io downloads.
Remember about 3 platforms (unix, windows, macos) when manage dependencies.
Prefer dependencies with static linking option.

## Git

Use git worktrees and branches to isolate work: try an approach out in a branch, keep the main line stable.
In commits use Conventional commits (`docs/conventional-commits.md`). To preserve style, use `git-cliff`(git-cliff.org)
CLI and configs (it is installed)
CHANGELOG.md is generated (commit-based, see CONTRIBUTING.md): a post-commit hook (`.githooks`,
enable with `git config core.hooksPath .githooks`) updates it automatically; never edit it by hand,
the hook amends commits with the refreshed file. History rewrites (rebase, cherry-pick) skip the hook.
Follow semver 2.0 for versions. (`docs/semantic-versioning.md`)

## Self-improvement

Record reusable commands, workflows and gotchas here, and keep this file accurate as the project
evolves. Move longer material into `docs/` and link it from here.

## Useful commands

Cross check the macOS build from Windows (no linking, FFI declarations included):

```powershell
rustup target add aarch64-apple-darwin
cargo check --target aarch64-apple-darwin --all-targets
```

Unix code paths (xkb layout detection, its tests) compile only on unix, so test them in a Linux
environment (container, VM, WSL); the WSL form, where the repo sits under `/mnt`:

```powershell
wsl -d <distro> bash -lc "cd <repo root in the /mnt/... form> && cargo test --lib"
```

Regenerate the shipped xkb keymap tables (needs the Linux xkb data, so run it in a Linux environment):

```powershell
wsl -d <distro> bash -lc "cd <repo root in the /mnt/... form> && python3 scripts/gen-keymap-tables.py"
```

Check what the layout probing found on the current machine: `silv --key-map-show`.

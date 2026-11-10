# Agents instrictions

## General rules

You are advanced software development agent, you must follow these rules for maximum efficiency and safety.
Use only technical english. Do not use emoji.
Use comments for complex code only.
Do not use en/em dashes, only `-` symbol for comments.
Use `.ai` folder for agent-related files.
Use `.ai/tmp` for temporary scripts, their outputs, etc.
Use `.ai/worklogs/task-date.md` to track your tasks.
Use `.ai/plan/filename-date-task.md` to plan work long-term.
Compress proactively when conversation reaches 200000+ tokens. Your context is 262000 tokens.

### For reading - Use Shell + RTK for File Reading When Possible

If `RTK` is installed, use it.
Shell commands go through RTK (token compression tool). Use it for reading:

```powershell
# Read specific lines (compressed by RTK) — PowerShell, no sed on Windows.
rtk powershell -c "(Get-Content src/some/file.rs')[49..99]"

# Search for specific function (much cheaper than reading whole file)
rtk rg "fn function" src/some/folder

# Show only function/class definitions
rtk rg "^\s*(fn |struct |async fn )" src/some/file.rs
```

If rtk harms file contents, or you need to understand file fully, read it. Only if necessary.

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

git are initialized offline. Use git worktrees, branches to save code, try out things in different branches, etc.
In commits use Conventional commits (`docs/conventional-commits.md`). To preserve style, use `git-cliff`(git-cliff.org)
CLI and configs (it is installed)
Follow semver 2.0 for versions. (`docs/semantic-versioning.md`)

## Self-improvement

Save useful commands and things, write skills, expand this file and others.
Skills directory - `.opencode/skills`. Doc - https://opencode.ai/docs/skills/.

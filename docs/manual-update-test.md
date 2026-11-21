# Manually testing the update flow

`scripts/mock-update-api.py` is a stdlib-only mock of the github releases api. It lets you run
the whole self-update flow against localhost: the check, the changelog, the download, the
self-replace, and the unreachable-repo pop-up. Python 3, no dependencies.

The mock answers the routes the updater uses - `/repos/{owner}/{repo}/releases` (the listing),
`/repos/{owner}/{repo}/releases/tags/{tag}` (the single release the pinned install asks for) and
the asset download - and serves two fake releases: `v9.9.9` and `v0.4.0`, so the changelog has
something to show. The "new binary" asset defaults to `target/debug/silv[.exe]`, so a self-update
lands on a runnable binary.

## 1. Build and start the mock

```powershell
cargo build
python scripts/mock-update-api.py --port 8080
```

Keep that terminal open (it logs every request). The repo url silv should be pointed at is printed
in its banner: `http://127.0.0.1:8080/testowner/testrepo`. Options: `--version`,
`--extra-version`, `--prerelease <version>` (publishes a release candidate above the newest stable
one, see section 7), `--asset <file>`, `--triple <target>`.

## 2. CLI: check

```powershell
target\debug\silv.exe update --check --repo-url http://127.0.0.1:8080/testowner/testrepo
```

Expected:

```
silv: checking releases at http://127.0.0.1:8080/testowner/testrepo (running v0.3.0)
update available: silv 0.3.0 -> 9.9.9

changelog, the releases between silv 0.3.0 and 9.9.9:
  v9.9.9 (2026-11-20)
    ...

run `silv update` to install it
```

Exit code 1 (an update was found; grep-like: 0 = up to date, 2 = trouble), everything on stderr.
Point `--repo-url` at a closed port to see exit 2 with `cannot read releases at ...`.

## 3. CLI: perform (on a copy, not on your build)

```powershell
Copy-Item target\debug\silv.exe $env:TEMP\silv-copy.exe
& $env:TEMP\silv-copy.exe update --repo-url http://127.0.0.1:8080/testowner/testrepo
(Get-FileHash $env:TEMP\silv-copy.exe).Hash   # equals the hash of target\debug\silv.exe
& $env:TEMP\silv-copy.exe --version           # runs the replaced binary
Remove-Item $env:TEMP\silv-copy.exe
```

Expected: `silv updated to 9.9.9; restart silv to use it`, exit 0. The copy becomes
byte-identical to the served asset. The served asset is your current debug build, so the
replaced copy still reports `0.3.0` - that is expected, the version string comes from the
binary you served, not from the release tag. A failed install (unwritable directory,
asset-less release) exits 2 with `update failed: ...`.

## 3a. Prove the replacement is functional, not just byte equality

With the default asset (the current build) a replace only changes the hash. To watch the
program itself change, build a visually distinct "new" binary, serve it, and update a stock
copy from it:

```powershell
# 1. Build the "new" release: uppercase the first line of --help (the LONG_ABOUT constant
#    in src/cli.rs, "Reads JSON Lines..." -> "READS JSON Lines..."), keep it, revert, rebuild:
mkdir .ai\tmp -Force
cargo build
Copy-Item target\debug\silv.exe .ai\tmp\silv-new.exe
git checkout -- src/cli.rs
cargo build                       # stock "old" binary again

# 2. Serve the "new" one as the release asset (mock terminal, port 8080):
python scripts/mock-update-api.py --port 8080 --asset .ai\tmp\silv-new.exe

# 3. Update a stock copy from it:
Copy-Item target\debug\silv.exe $env:TEMP\silv-e2e.exe
& $env:TEMP\silv-e2e.exe --help | Select-Object -First 1   # Reads JSON Lines ...
& $env:TEMP\silv-e2e.exe update --repo-url http://127.0.0.1:8080/testowner/testrepo
& $env:TEMP\silv-e2e.exe --help | Select-Object -First 1   # READS JSON Lines ...
(Get-FileHash $env:TEMP\silv-e2e.exe).Hash                 # equals .ai\tmp\silv-new.exe
Remove-Item $env:TEMP\silv-e2e.exe, .ai\tmp\silv-new.exe
```

Expected: the first `--help` line flips `Reads` -> `READS` after the update. The already
running old copy keeps behaving like the old binary while it works; only the file on disk is
swapped (self-replace replaces the path, not the running image). The automated equivalent is
`tests/update_mock.rs::update_performs_self_replace_on_a_temp_copy`: it serves
`<the binary under test> + marker bytes`, asserts the copy equals the served asset (not the
runner), and executes the replaced copy - trailing data keeps a PE/ELF executable.

## 4. TUI: the update line and the changelog

```powershell
echo '{"message":"hello","level":"INFO"}' > $env:TEMP\t.jsonl
target\debug\silv.exe $env:TEMP\t.jsonl --repo-url http://127.0.0.1:8080/testowner/testrepo
```

At startup silv checks releases in the background. Within a second the yellow update button
appears at the very bottom. Its full text is:

```
update available: 0.3.0 -> 9.9.9 (click to update, to disable checks use "--no-update" or "SILV_NO_UPDATE=true")
```

On a narrow terminal it is cut with `..` in front of the `changelog` button, so the hint stays
visible as an option:

```
update available: 0.3.0 -> 9.9.9 (click to update, to disable checks use ..   changelog
```

- Click the left button: the update runs (the check thread prints progress), the line disappears,
  the status bar shows `update: silv updated to 9.9.9, restart silv to use it`.
  This replaces `target\debug\silv.exe` itself - rebuild afterwards if you want it back.
- Keyboard: `Tab` cycles from the rows to the update line (the focused button is the brighter one),
  `Left`/`Right` choose between the update and the changelog button, `Enter` presses the focused
  one, `Esc` returns to the rows. The line is skipped by the cycle while it is not shown.
- Click `changelog`: the overlay shows the notes of v9.9.9 and v0.4.0; `j/k` scroll, `f/b` page,
  `g/G` ends, wheel works, `q`/`Esc` close. The last scroll position shows the last full page.
- Clicks on the update line behind an open overlay (changelog, record detail, value picker) are
  inert; while an update downloads, `q` asks for confirmation ("press q again") once.
- The mock request log shows what silv asked for: the listing, then `/releases/tags/v9.9.9`
  (the pinned release) and the asset download on a perform.

## 5. TUI: forks and unreachable repos

- Press `u`: the repository url pop-up opens. Enter `testowner/testrepo`... that is github, not
  the mock, so it will report unreachable. Enter the full mock url
  `http://127.0.0.1:8080/testowner/testrepo` and press Enter: the check re-runs against the mock
  and the update line appears. Submitting a url while a check is still in flight is not lost: the
  running check belongs to the old repository, its answer is dropped, the new one takes over.
- Esc closes the pop-up, like every other window; clicks outside it are inert (you cannot lose
  typed text by clicking around), and the mouse wheel does not scroll the rows behind it.
  Submitting while the update line was focused returns the focus to the rows (the line hides
  during the re-check).
- Stop the mock server (Ctrl-C), press `u`, enter the mock url: silv reports it is unreachable and
  keeps the pop-up open for a new url. The same happens automatically at startup when the
  configured repo is unreachable.
- `--no-update` (or `SILV_NO_UPDATE=true`) with a mock url: no startup check, no update line,
  `u` does nothing.

## 6. Up to date

Restart the mock with an old version, then check:

```powershell
python scripts/mock-update-api.py --port 8080 --version 0.0.1 --extra-version 0.0.2
target\debug\silv.exe update --check --repo-url http://127.0.0.1:8080/testowner/testrepo
```

Expected: `silv v0.3.0 is up to date`, exit 0; in the TUI no update line appears at all, and a
line painted by an earlier check in the same session is cleared when the re-check answers
up to date.

## 7. Prereleases are never offered

A release train publishes release candidates between stables; silv must offer and install only
the newest stable release:

```powershell
python scripts/mock-update-api.py --port 8080 --version 9.9.9 --extra-version 9.9.8 --prerelease 9.9.10-rc.1
target\debug\silv.exe update --check --repo-url http://127.0.0.1:8080/testowner/testrepo
```

Expected: the report offers `9.9.9` and exits 1; `9.9.10-rc.1` and its notes are absent.
`silv update` (on a copy, as in section 3) installs `9.9.9` - the mock log shows the pinned
request `/repos/testowner/testrepo/releases/tags/v9.9.9`, never the rc. A listing where only a
prerelease is newer reports `silv v0.3.0 is up to date` and exits 0. The automated versions are
`check_skips_prerelease_releases`, `check_is_up_to_date_when_only_prereleases_are_newer` and the
rc-above-stable listing of `update_performs_self_replace_on_a_temp_copy` in `tests/update_mock.rs`.

The pinned install protects the rc rule on forks too: a repository tagging `9.9.8` (no `v`) is
served by the bare-tag retry, and a repository whose newest release overall is an rc has the
listing-based install refused - exit 2, binary untouched, never an rc
(`update_installs_a_fork_that_tags_releases_without_the_v_prefix`,
`update_refuses_the_listing_fallback_when_a_prerelease_is_newer`).

## Notes

- The asset name in the listing is `silv-<tag>-<target>[.exe]` for your machine's triple
  (same names as `.github/workflows/release.yml`); the updater picks the asset matching its own
  target, so the mock must run on the machine being updated (or pass `--triple`).
- `GITHUB_TOKEN` is not needed: the api here is unauthenticated localhost. It is never sent
  here anyway - the token only travels to the default api.github.com (asserted by
  `the_github_token_never_travels_to_a_custom_repo_url`).
- The automated version of all this is `tests/update_mock.rs` (same mock, in-process).
- A binary installed by `cargo install` (in `$CARGO_HOME/bin`, `~/.cargo/bin` by default) is
  detected: the update line says `update using cargo` instead of `click to update`, and pressing
  the button prints the `cargo install --git <url> --force` command - no self-update runs there, it
  would desync cargo's install list (verified by hand: after a self-update of a cargo binary,
  `~/.cargo/.crates2.json` still names the old version, and `cargo install --git <url>` at that
  version says "already installed, use --force"). The CLI mirrors the refusal: `silv update` exits
  2 with the cargo command, `silv update --force` performs the replace. Try the detection with
  `cargo install --path .`, then start `silv`.

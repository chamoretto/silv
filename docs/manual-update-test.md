# Manually testing the update flow

`scripts/mock-update-api.py` is a stdlib-only mock of the github releases api. It lets you run
the whole self-update flow against localhost: the check, the changelog, the download, the
self-replace, and the unreachable-repo pop-up. Python 3, no dependencies.

The mock answers the two routes the updater uses (`/repos/{owner}/{repo}/releases` and the asset
download) and serves two fake releases: `v9.9.9` and `v0.4.0`, so the changelog has something to
show. The "new binary" asset defaults to `target/debug/silv[.exe]`, so a self-update lands on a
runnable binary.

## 1. Build and start the mock

```powershell
cargo build
python scripts/mock-update-api.py --port 8080
```

Keep that terminal open (it logs every request). The repo url silv should be pointed at is printed
in its banner: `http://127.0.0.1:8080/testowner/testrepo`. Options: `--version`,
`--extra-version`, `--asset <file>`, `--triple <target>`.

## 2. CLI: check

```powershell
target\debug\silv.exe update --check --repo-url http://127.0.0.1:8080/testowner/testrepo
```

Expected: `update available: silv 0.3.0 -> 9.9.9`, the release notes of both mock releases,
`run 'silv update' to install it`, exit code 0.

## 3. CLI: perform (on a copy, not on your build)

```powershell
Copy-Item target\debug\silv.exe $env:TEMP\silv-copy.exe
& $env:TEMP\silv-copy.exe update --repo-url http://127.0.0.1:8080/testowner/testrepo
(Get-FileHash $env:TEMP\silv-copy.exe).Hash   # equals the hash of target\debug\silv.exe
& $env:TEMP\silv-copy.exe --version           # runs the replaced binary
Remove-Item $env:TEMP\silv-copy.exe
```

Expected: `silv updated to 9.9.9; restart silv to use it`. The copy becomes byte-identical to the
served asset. The served asset is your current debug build, so the replaced copy still reports
`0.3.0` - that is expected, the version string comes from the binary you served, not from the
release tag.

## 4. TUI: the update line and the changelog

```powershell
echo '{"message":"hello","level":"INFO"}' > $env:TEMP\t.jsonl
target\debug\silv.exe $env:TEMP\t.jsonl --repo-url http://127.0.0.1:8080/testowner/testrepo
```

At startup silv checks releases in the background. Within a second the bottom of the screen shows:

```
 update available: silv 0.3.0 -> 9.9.9 (click to update)                changelog
```

- Click the left button: the update runs (the check thread prints progress), the line disappears,
  the status bar shows `update: silv updated to 9.9.9, restart silv to use it`.
  This replaces `target\debug\silv.exe` itself - rebuild afterwards if you want it back.
- Click `changelog`: the overlay shows the notes of v9.9.9 and v0.4.0; `j/k` scroll, `f/b` page,
  `g/G` ends, wheel works, `Esc` closes.
- The mock request log shows what silv asked for.

## 5. TUI: forks and unreachable repos

- Press `u`: the repository url pop-up opens. Enter `testowner/testrepo`... that is github, not
  the mock, so it will report unreachable. Enter the full mock url
  `http://127.0.0.1:8080/testowner/testrepo` and press Enter: the check re-runs against the mock
  and the update line appears.
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

Expected: `silv v0.3.0 is up to date`; in the TUI no update line appears at all.

## Notes

- The asset name in the listing is `silv-<tag>-<target>[.exe]` for your machine's triple
  (same names as `.github/workflows/release.yml`); the updater picks the asset matching its own
  target, so the mock must run on the machine being updated (or pass `--triple`).
- `GITHUB_TOKEN` is not needed: the api here is unauthenticated localhost.
- The automated version of all this is `tests/update_mock.rs` (same mock, in-process).

# silv - simple interactive log viewer

SILV reads JSON Lines and `timestamp | LEVEL | message key=value` logs and shows them in a
Kibana-like layout. Two modes share the same visuals:

- **Interactive TUI** (default when stdin and stdout are terminals): sticky search bar, filter
  chips, sticky field header, scrollable rows table, full-screen record detail with field actions.
- **Stream mode** (default when piped): grep-like colored output, fields table on top, driven by
  CLI flags. Exit code `1` when nothing matched, like `grep`.

Single statically linked binary, drop it on a server over SFTP and run it.

## Build

```sh
cargo build --release            # target/release/silv
scripts/build-release.ps1        # windows release (dist/)
scripts/build-release.sh         # unix releases (dist/)
```

Requires Rust stable (edition 2024). All dependencies are pure Rust with exact `=x.y.z` pins, so
fully static builds work on windows (MSVC CRT static) and linux (musl).

## Quick start

```sh
silv app.jsonl                                   # interactive, whole file
silv -p app.jsonl -f @timestamp,@level,message --fields-non-ordered-last
silv -p app.jsonl --filter-neq message.logger "aiokafka.*" --search timeout
cat app.jsonl | silv -p --filter-eq level WARN --render kv
silv -p app.jsonl --filter 'level=WARN and not message.logger=app.payments'
silv -i logs/ --include '*.jsonl' -r --max-records 100000
```

## Input

- Files and directories as positional arguments; a lone `-` (or no arguments) reads stdin.
- Directories are scanned non-recursively unless `-r`; `--include`/`--exclude` globs match the
  path and the file name.
- Format auto-detection per source: a first JSON object line means JSON Lines, a
  `timestamp | LEVEL | message` shape means the plain stdout format. Force with
  `--format jsonl|plain`.
- Large files (>= 8 MiB) are memory mapped; parsing is parallel per line. All records are kept in
  RAM (the price of the Kibana experience), capped by `--max-records` (default 500 000).
- Lines that fail to parse become `INVALID` records and stay visible; `--no-keep-bad-lines` drops
  them, `--strict` aborts with a miette diagnostic pointing at the offending line.

## Fields

Field paths address values inside each record: `a.b`, quoted keys `a."b.c"`, array indexes
`a[2]`, wildcards `a[*]`. Pseudo fields: `@timestamp` (`@ts`), `@level` (`@severity`),
`@message` (`@msg`), `@source` (`@file`), `@line` (`@n`).

Alias resolution for the timestamp/level/message pseudo fields: `--timestamp-field`,
`--level-field`, `--message-field` add paths checked before the built-in defaults
(`@timestamp, timestamp, ts, time, time_ms, eventTime, datetime, date, _ts, severityText,
severity, sev, level, status, severity_number, log.level, msg, message, body, event, event.dataset,
status_message`).

Level parsing understands names (with aliases like `WARNING`/`ERR`/`FATAL`), syslog numbers
(0-7), OTel numbers (8-24) and log4j/python/logback style numbers.

## Columns and rendering

- `-f, --fields-order ts,level,message` selects ordered columns (repeatable, comma separated).
- `--fields-non-ordered-last` appends every remaining discovered field (flattened to leaf paths)
  after the ordered ones. `--all-fields` uses all of them, `--exclude-fields` never shows some.
- `--render auto|table|kv|line|json|jsonl`:
  - `auto`: table when columns are selected, otherwise the format default (kv for JSON Lines,
    the source line for the plain format);
  - `table`: aligned columns with a header row (widths sampled, fitted to `--width`);
  - `kv`: `timestamp LEVEL message key=value key=value ...` per record, alias-covered fields
    are not repeated;
  - `line`: the original source line;
  - `json` / `jsonl`: pretty / compact JSON of the record object.
- Numeric columns right-align; `--truncate-values N` caps cell text (default 120, `0` disables);
  `--no-headers` drops the header row; `--width` overrides the detected terminal width.

## Filtering and search

Patterns are globs (`*`, `?`, `[a-z]`, `{a,b}`) matched against the text form of a field, anchored
at both ends; `--glob-anywhere` wraps them as `*pattern*`. `--filter-regex` switches pattern
syntax to regular expressions. `neq` filters pass when the field is missing.

Stream mode flags (all repeatable, combined with AND):

```text
-s, --search PATTERN          free text; every pattern must match somewhere in the record
    --search-fields a,b       restrict the search to these fields
    --search-regex            search patterns are regular expressions
    --case-sensitive           case sensitive matching (default is insensitive)
    --filter 'EXPR'           full DSL expression (see below)
    --filter-eq FIELD PATTERN field value equals a glob (also accepts FIELD=PATTERN)
    --filter-neq FIELD PATTERN field value differs from a glob
    --filter-exists FIELD     field must exist
    --filter-not-exists FIELD field must be absent
```

Filter DSL grammar:

```text
expr    := or
or      := and ('or' | '||' and)*
and     := not ('and' | '&&' not)*
not     := 'not' | '!' | unary
unary   := '(' expr ')' | leaf
leaf    := FIELD ('=' | '!=' | '~' | '!~' | '^' | '!^') PATTERN
```

`=` / `!=` compare the text form of the value against a glob (or regex with
`--filter-regex`), `~` / `!~` scan every string inside containers for a substring,
`^` / `!^` test existence. Example:
`--filter 'level=WARN* and (app.env=prod or app.env=staging) and not message.logger=aiokafka.*'`.

Interactive mode exposes the same predicates as chips: `/` search, `F` add filter (the input
accepts the DSL), chips are deleted with `x`/`d`.

## Interactive key map

```text
/            focus search            F            add a filter
Tab/BackTab  move focus              Enter        open record / apply input / sort focused field
j k g G Pg   move row                s            sort focused field (asc, desc, off)
c            field picker            d            delete focused filter, or focused column in header
v            cycle render mode       t m          toggle timestamp / message column
L r A        case, regex, scope      x X          clear search, clear all filters
Mouse        click row = detail, click header = sort, wheel = scroll
?            help                    q, Ctrl-C    quit
```

In the record detail overlay: `a` toggle column, `f`/`F` eq/neq filter from a value, `~` contains,
`e`/`E` exists/missing, `v` toggle JSON, `Enter` field action menu, `Esc` close.

## Colors and output

- Colors follow the terminal: auto when stdout is a TTY, disabled by `NO_COLOR`, `TERM=dumb`,
  pipes or redirection. Force with `--color always|never` / `--no-color`.
- Severity, keys, punctuation, numbers, containers and hits each get their own style; search
  hits are highlighted (reverse video) in table and kv output.
- `--stats` prints parse counts, source list and bad-line summaries to stderr.

## Exit codes

```text
0  records matched (or interactive session ended)
1  no records matched (stream mode, grep compatible) or a runtime/diagnostic error
2  command line usage error (clap)
```

## Diagnostics

Errors are rendered by miette with stable codes and source spans:
`silv::io::read`, `silv::io::write`, `silv::input::empty`, `silv::input::format_undetected`,
`silv::parse::invalid_line` (points at the exact line), `silv::field::path_invalid`,
`silv::filter::expression_invalid`, `silv::filter::glob_invalid`, `silv::filter::regex_invalid`,
`silv::time::format_invalid`, `silv::app::not_a_terminal`, `silv::cli::argument_invalid`.

```text
Error: silv::parse::invalid_line

  x failed to parse log records
  `- line 14: line is not a JSON object
    ,- [samples/sample.jsonl:14:1]
 13 | {"ts":"2025-11-09T07:15:11.640Z", ... }
 14 | not a json line at all
    : ^^^^
    `----
  help: check the reported line; use `--no-keep-bad-lines` to stop on the first bad line
```

## Static binaries (drop-in SFTP deployment)

| target | command | linkage |
|---|---|---|
| `x86_64-unknown-linux-musl` | `cargo build --release --target x86_64-unknown-linux-musl` (with `target.x86_64-unknown-linux-musl.rustflags = ["-C","target-feature=+crt-static"]`) | fully static, runs on any ubuntu/debian |
| `x86_64-pc-windows-msvc` | `cargo build --release` (with `target.x86_64-pc-windows-msvc.rustflags = ["-C","target-feature=+crt-static"]`) | static CRT; needs the usual VC runtime present on all modern Windows |
| `aarch64-apple-darwin` | `cargo build --release` | system libc/libresolv must stay dynamic (Apple forbids fully static); the result runs on every Apple Silicon mac |

Everything SILV uses (globset, regex, ratatui/crossterm, serde_json, clap, miette, rayon,
memmap2, unicode-width) is pure Rust, so the linux-musl target produces a real ELF with zero
dynamic dependencies.

`.cargo/config.toml` template (project-local; keep it out of the repo if you prefer env vars):

```toml
[target.x86_64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]

[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

`scripts/build-release.ps1` / `scripts/build-release.sh` build the release binary (and on unix
also the musl build when the target is installed) into `dist/`.

## Development

```sh
cargo test --all-targets        # 129 unit + 22 end-to-end CLI tests
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Roadmap: `.ai/plan/2026-11-10-silv-v0.1.0.md`; worklogs: `.ai/worklogs/`.

## License

MIT. See `LICENSE`.

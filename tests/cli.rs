//! End-to-end CLI tests: run the built binary against the sample fixtures.

use std::io::Write;
use std::process::Command;
use std::process::ExitStatus;

fn jsonl_path() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/samples/sample.jsonl").to_owned()
}

fn plain_path() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/samples/sample.loglines").to_owned()
}

struct Run {
    code: ExitStatus,
    out: String,
    err: String,
}

impl Run {
    fn out_lines(&self) -> Vec<String> {
        self.out.lines().map(str::to_owned).collect()
    }

    fn must_ok(&self) -> &Self {
        assert!(self.code.success(), "expected success, got {:?}\nstderr: {}", self.code, self.err);
        self
    }

    fn must_fail(&self) -> &Self {
        assert!(!self.code.success(), "expected failure\nstdout: {}\nstderr: {}", self.out, self.err);
        self
    }
}

fn run_stdin(args: &[&str], stdin_data: Option<&[u8]>) -> Run {
    let bin = env!("CARGO_BIN_EXE_silv");
    let mut child = Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawns silv");
    {
        let stdin = child.stdin.as_mut().expect("stdin piped");
        match stdin_data {
            Some(data) => stdin.write_all(data).expect("writes stdin"),
            None => stdin.write_all(b"").expect("writes empty stdin"),
        }
    }
    let output = child.wait_with_output().expect("waits silv");
    Run {
        code: output.status,
        out: String::from_utf8_lossy(&output.stdout).into_owned(),
        err: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run(args: &[&str]) -> Run {
    run_stdin(args, None)
}

#[test]
fn fields_table_with_glob_filter() {
    let path = jsonl_path();
    let case = run(&[
        "-p",
        &path,
        "-f",
        "ts,level,message",
        "--fields-non-ordered-last",
        "--filter-neq",
        "message.logger",
        "aiokafka.*",
        "--width",
        "400",
    ]);
    case.must_ok();
    let header = case.out_lines().first().cloned().unwrap_or_default();
    assert!(header.contains("ts"), "header: {header}");
    assert!(header.contains("level"), "header: {header}");
    // Ordered fields come first, the flattened remainder of every record follows.
    assert!(header.contains("trace"), "header: {header}");
    // app.* records survive (values at MIN column width stay readable),
    // aiokafka.* records are filtered out by the glob.
    assert!(case.out.contains("web1"), "app record missing: {}", case.out);
    assert!(case.out.contains("A-1021"), "app record missing: {}", case.out);
    assert!(!case.out.contains("deseri"), "aiokafka record leaked: {}", case.out);
    assert!(!case.out.contains("billin"), "aiokafka record leaked: {}", case.out);
}

#[test]
fn fields_order_stays_readable_without_remainder() {
    let path = jsonl_path();
    let case = run(&[
        "-p",
        &path,
        "-f",
        "ts,level,message.logger,message.text",
        "--filter-neq",
        "message.logger",
        "aiokafka.*",
        "--width",
        "120",
    ]);
    case.must_ok();
    assert!(case.out.contains("worker started"), "{}", case.out);
    assert!(case.out.contains("charge failed"), "{}", case.out);
    assert!(case.out.contains("app.orders"), "{}", case.out);
    assert!(!case.out.contains("socket close in activity"), "aiokafka leaked: {}", case.out);
    assert!(!case.out.contains("metadata updated"), "aiokafka leaked: {}", case.out);
}

#[test]
fn fields_non_ordered_last_flattens_remainder() {
    let path = jsonl_path();
    let case = run(&[
        "-p",
        &path,
        "-f",
        "ts,level,message.logger,message.text",
        "--fields-non-ordered-last",
        "--width",
        "400",
    ]);
    case.must_ok();
    let lines = case.out_lines();
    let header = lines.first().cloned().unwrap_or_default();
    // Ordered columns first, then the discovered remainder of every record.
    let head = header.replacen("ts", "", 1);
    assert!(head.contains("messa"), "header: {header}");
    assert!(lines.iter().any(|l| l.contains("A-1021") && l.contains("249.99")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("128.5") && l.contains("3600")), "meta remainder missing");
}

#[test]
fn search_narrows_kv_output() {
    let path = jsonl_path();
    let case = run(&["-p", "--render", "kv", "--search", "order accepted", &path]);
    case.must_ok();
    assert_eq!(case.out_lines().len(), 2, "stdout: {}", case.out);
    assert!(case.out.contains("A-1021") && case.out.contains("A-1022"), "{}", case.out);
    assert!(!case.out.contains("probe ok"), "{}", case.out);
}

#[test]
fn eq_filter_from_stdin() {
    let data = std::fs::read(jsonl_path()).expect("fixture readable");
    let case = run_stdin(&["-p", "--filter-eq", "level", "WARN"], Some(&data));
    case.must_ok();
    assert_eq!(case.out_lines().len(), 1, "stdout: {}", case.out);
    assert!(case.out.contains("socket close"), "{}", case.out);
}

#[test]
fn no_match_exits_one() {
    let path = jsonl_path();
    let case = run(&["-p", "-s", "zzz-nothing", &path]);
    case.must_fail();
    assert_eq!(case.code.code(), Some(1), "stderr: {}", case.err);
}

#[test]
fn strict_reports_the_offending_line() {
    let path = jsonl_path();
    let case = run(&["-p", "--strict", &path]);
    case.must_fail();
    assert!(case.err.contains("silv::parse::invalid_line"), "stderr: {}", case.err);
    assert!(case.err.contains("not a JSON object"), "stderr: {}", case.err);
    assert!(case.err.contains("not a json line at all"), "stderr: {}", case.err);
}

#[test]
fn plain_format_renders_source_lines() {
    let path = plain_path();
    let case = run(&["-p", &path]);
    case.must_ok();
    assert!(
        case.out
            .contains("2025-11-09 07:15:01.120 | INFO | worker started host=web1"),
        "{}",
        case.out
    );
    assert!(case.out.contains("topic=\"orders in\""), "{}", case.out);
}

#[test]
fn plain_format_supports_pseudo_columns() {
    let path = plain_path();
    let case = run(&["-p", &path, "-f", "@timestamp,@level,@message,order_id"]);
    case.must_ok();
    let header = case.out_lines().first().cloned().unwrap_or_default();
    assert!(header.contains("@timestamp") && header.contains("@level"), "header: {header}");
    assert!(case.out.contains("worker started"), "{}", case.out);
    assert!(case.out.contains("A-1021"), "{}", case.out);
}

#[test]
fn tail_keeps_last_records() {
    let path = jsonl_path();
    let case = run(&["-p", "--render", "kv", "--tail", "2", &path]);
    case.must_ok();
    assert_eq!(case.out_lines().len(), 2, "stdout: {}", case.out);
    assert!(case.out.contains("order shipped"), "{}", case.out);
}

#[test]
fn jsonl_render_is_compact_json() {
    let path = jsonl_path();
    let case = run(&["-p", "--render", "jsonl", "--tail", "1", &path]);
    case.must_ok();
    let line = case.out_lines().first().cloned().unwrap_or_default();
    assert!(line.starts_with('{') && line.ends_with('}'), "line: {line}");
    assert!(line.contains("\"carrier\": \"dhl\""), "line: {line}");
    assert!(!line.contains('\n'), "single line expected");
}

#[test]
fn json_render_is_pretty() {
    let path = jsonl_path();
    let case = run(&["-p", "--render", "json", "--tail", "1", &path]);
    case.must_ok();
    assert!(case.out_lines().len() > 3, "pretty json expected:\n{}", case.out);
    assert!(case.out.contains("\"carrier\""), "{}", case.out);
}

#[test]
fn filter_dsl_composites() {
    let path = jsonl_path();
    let case = run(&[
        "-p",
        "--render",
        "kv",
        "--filter",
        "message.logger=app.orders* and message.order_id=A-1021",
        &path,
    ]);
    case.must_ok();
    assert!(case.out.contains("order accepted"), "{}", case.out);
    assert!(case.out.contains("refund queued"), "{}", case.out);
    assert!(!case.out.contains("order shipped"), "{}", case.out);

    let negated = run(&["-p", "--render", "kv", "--filter", "not message.logger=app.*", &path]);
    negated.must_ok();
    assert!(!negated.out.contains("worker started"), "app.* leaked: {}", negated.out);
    assert!(negated.out.contains("socket close"), "{}", negated.out);
}

#[test]
fn exists_filters_select_records() {
    let path = jsonl_path();
    let case = run(&["-p", "--render", "kv", "--filter-exists", "trace.id", &path]);
    case.must_ok();
    assert_eq!(case.out_lines().len(), 2, "stdout: {}", case.out);
    assert!(case.out.contains("charge failed") && case.out.contains("refund queued"), "{}", case.out);
}

#[test]
fn stats_goes_to_stderr_only() {
    let path = jsonl_path();
    let case = run(&["-p", "--stats", "--tail", "1", &path]);
    case.must_ok();
    assert!(case.err.contains("parsed 16 record(s)"), "stderr: {}", case.err);
    assert!(case.err.contains("bad line"), "stderr: {}", case.err);
    assert!(!case.out.contains("parsed"), "stats leaked to stdout: {}", case.out);
}

#[test]
fn bad_lines_are_kept_by_default_and_droppable() {
    let path = jsonl_path();
    let kept = run(&["-p", "--render", "kv", &path]);
    kept.must_ok();
    assert!(kept.out.contains("not a json line at all"), "bad line dropped: {}", kept.out);

    let dropped = run(&["-p", "--render", "kv", "--no-keep-bad-lines", &path]);
    dropped.must_ok();
    assert!(!dropped.out.contains("not a json line at all"), "bad line kept: {}", dropped.out);
}

#[test]
fn color_flags_emit_ansi() {
    let path = jsonl_path();
    let plain = run(&["-p", "--color", "never", "--tail", "1", &path]);
    plain.must_ok();
    assert!(!plain.out.contains('\u{1b}'), "unexpected escape: {}", plain.out);

    let colored = run(&["-p", "--color", "always", "--tail", "1", &path]);
    colored.must_ok();
    assert!(colored.out.contains('\u{1b}'), "no escape codes in colored output");
}

#[test]
fn sort_by_field_orders_rows() {
    let path = jsonl_path();
    let asc = run(&[
        "-p",
        "--render",
        "kv",
        "--sort",
        "message.order_id",
        "--filter-exists",
        "message.order_id",
        &path,
    ]);
    asc.must_ok();
    let first = asc.out_lines().first().cloned().unwrap_or_default();
    assert!(first.contains("A-1021"), "first row: {first}");

    let desc = run(&[
        "-p",
        "--render",
        "kv",
        "--sort",
        "-message.order_id",
        "--filter-exists",
        "message.order_id",
        &path,
    ]);
    desc.must_ok();
    let first = desc.out_lines().first().cloned().unwrap_or_default();
    assert!(first.contains("A-1022"), "first row: {first}");
}

#[test]
fn include_glob_selects_files() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/samples");
    let case = run(&["-p", dir, "--include", "*.jsonl", "--tail", "1"]);
    case.must_ok();
    assert!(case.out.contains("order shipped"), "{}", case.out);

    let excluded = run(&["-p", dir, "--include", "*.loglines"]);
    excluded.must_ok();
    assert!(!excluded.out.contains("order shipped"), "jsonl leaked: {}", excluded.out);
    assert!(excluded.out.contains("worker started"), "{}", excluded.out);
}

#[test]
fn empty_input_fails_cleanly() {
    let case = run_stdin(&["-p"], Some(b""));
    case.must_fail();
    assert!(case.err.contains("silv::input::empty"), "stderr: {}", case.err);
}

#[test]
fn missing_file_reports_read_error() {
    let case = run(&["-p", "no-such-file.jsonl"]);
    case.must_fail();
    assert!(case.err.contains("silv::io::read"), "stderr: {}", case.err);
}

#[test]
fn bad_filter_reports_dsl_diagnostic() {
    let path = jsonl_path();
    let case = run(&["-p", "--filter", "((bad", &path]);
    case.must_fail();
    assert_eq!(case.code.code(), Some(1), "stderr: {}", case.err);
    assert!(case.err.contains("silv::filter::expression_invalid"), "stderr: {}", case.err);
}

#[test]
fn bad_field_path_reports_diagnostic() {
    let path = jsonl_path();
    let case = run(&["-p", "-f", "a..b", &path]);
    case.must_fail();
    assert!(case.err.contains("silv::field::path_invalid"), "stderr: {}", case.err);
}

#[test]
fn usage_errors_exit_two() {
    let case = run(&["--badflag"]);
    case.must_fail();
    assert_eq!(case.code.code(), Some(2));
}

#[test]
fn help_and_version() {
    let help = run(&["--help"]);
    help.must_ok();
    assert!(help.out.contains("--fields-non-ordered-last"), "{}", help.out);
    assert!(help.out.contains("--filter-neq"), "{}", help.out);
    assert!(help.out.contains("--key-map-show"), "{}", help.out);

    let version = run(&["--version"]);
    version.must_ok();
    assert!(version.out.contains("silv"), "{}", version.out);
}

#[test]
fn key_map_show_reports_the_resolved_map() {
    // Runs the platform probe path in CI, where a broken probe shows up as a failure of this test.
    let auto = run(&["--key-map-show"]);
    auto.must_ok();
    assert!(auto.out.contains("key map: auto/"), "{}", auto.out);
    assert!(auto.out.contains("entries: "), "{}", auto.out);

    let raw = run(&["--key-map", "raw", "--key-map-show"]);
    raw.must_ok();
    assert!(raw.out.contains("key map: raw (no mapping)"), "{}", raw.out);

    let builtin = run(&["--key-map", "ru", "--key-map-show"]);
    builtin.must_ok();
    assert!(builtin.out.contains("key map: layout table `ru`"), "{}", builtin.out);
}

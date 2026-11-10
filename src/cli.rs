//! Command line surface, argument validation and the resolved run configuration.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use clap::ValueEnum;
use miette::Result;

use crate::error::SilvError;
use crate::filter::LeafOp;
use crate::filter::Predicate;
use crate::filter::SearchScope;
use crate::filter::SearchSpec;
use crate::filter::build_leaf;
use crate::filter::dsl::FilterContext;
use crate::filter::parse_expression;
use crate::model::field::FieldPath;
use crate::model::record::FieldAliases;
use crate::model::timestamp::TimeFormat;
use crate::parse::LogFormat;
use crate::parse::plain::PlainOptions;

/// Records kept in memory by default, `--max-records` raises or lowers it.
pub const DEFAULT_MAX_RECORDS: usize = 500_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunMode {
    Interactive,
    Print,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormatSelection {
    Auto,
    Forced(LogFormat),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, ValueEnum)]
pub enum RenderChoice {
    /// Table when fields are selected, otherwise the format default.
    #[default]
    Auto,
    /// Aligned columns with a header row.
    Table,
    /// `key=value` line per record.
    Kv,
    /// Source line as-is (plain), compact JSON (jsonl).
    Line,
    /// Pretty JSON per record.
    Json,
    /// Compact JSON per record.
    Jsonl,
}

impl std::fmt::Display for RenderChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let token = match self {
            Self::Auto => "auto",
            Self::Table => "table",
            Self::Kv => "kv",
            Self::Line => "line",
            Self::Json => "json",
            Self::Jsonl => "jsonl",
        };
        f.write_str(token)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, ValueEnum)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, ValueEnum)]
pub enum MouseMode {
    /// Enable mouse capture except on consoles that leak mouse bytes (ConEmu class).
    #[default]
    Auto,
    /// Always capture mouse events.
    Enable,
    /// Never capture mouse events.
    Disable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, ValueEnum)]
pub enum FormatArg {
    #[default]
    Auto,
    Jsonl,
    Plain,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Asc => "^",
            Self::Desc => "v",
        }
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SortSpec {
    pub field: FieldPath,
    pub dir: SortDir,
}

/// Field/column selection for output.
#[derive(Clone, Debug, Default)]
pub struct ColumnSpec {
    pub ordered: Vec<FieldPath>,
    pub rest_last: bool,
    pub excluded: Vec<FieldPath>,
    pub all: bool,
}

const ABOUT: &str = "SILV - simple interactive log viewer (JSON Lines first, stdout-like logs second)";

const LONG_ABOUT: &str = "\
Reads JSON Lines or `timestamp | LEVEL | message key=value` logs and shows them in a Kibana-like layout.

Interactive mode (default when stdin and stdout are terminals): search bar, filter chips, sticky field header,
record detail view with actions.

Stream mode (default when piped): colored fields table on top, then records, driven by --fields-order,
--fields-non-ordered-last, --search and the --filter-* flags.

Patterns are globs (`*`, `?`, `[a-z]`, `{a,b}`) matched against the text form of a field, anchored at both ends.
Use --glob-anywhere to match the pattern anywhere inside the value.";

const EXAMPLES: &str = "\
Examples:
  silv app.jsonl
  silv -p app.jsonl -f @timestamp,@level,message --fields-non-ordered-last
  silv -p app.jsonl --filter-neq message.logger \"aiokafka.*\" --search timeout
  cat app.jsonl | silv -p --filter-eq level WARN --render kv
  silv -p app.jsonl --filter 'level=WARN and not msg~health'
  silv -i logs/ --include '*.jsonl' -r --max-records 100000";

const KEYS: &str = "\
Interactive keys:
  /            focus search bar         F            filter wizard: field, operator, value
  Tab / S-Tab  focus header cell        Enter        open record / apply input / sort focused column
  s            sort focused column      c            field picker (toggle columns)
  j k, PgUp..  move row                 g G          first / last row
  d            delete focused filter    x            clear search
  Esc          close overlay, clear search, quit     q, Ctrl-C  quit
  Mouse        click row = detail, click header = sort, wheel = scroll";

/// Raw arguments as parsed by clap.
#[derive(Parser, Debug, Clone)]
#[command(name = "silv", version, about = ABOUT, long_about = LONG_ABOUT, after_help = EXAMPLES)]
pub struct Cli {
    /// Files or directories to read; omit to read stdin, `-` also reads stdin
    #[arg(value_name = "FILES")]
    pub files: Vec<PathBuf>,

    /// Force the interactive TUI
    #[arg(short = 'i', long)]
    pub interactive: bool,

    /// Force stream (grep-like) output
    #[arg(short = 'p', long)]
    pub print: bool,

    /// Input format
    #[arg(long, value_enum, default_value = "auto")]
    pub format: FormatArg,

    /// Only read files matching this glob (repeatable)
    #[arg(long, value_name = "GLOB")]
    pub include: Vec<String>,

    /// Skip files matching this glob (repeatable)
    #[arg(long, value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Walk directories recursively
    #[arg(short = 'r', long)]
    pub recursive: bool,

    /// Ordered columns, comma separated (repeatable)
    #[arg(short = 'f', long = "fields-order", value_name = "FIELDS")]
    pub fields_order: Vec<String>,

    /// Append the remaining fields of each record after the ordered columns
    #[arg(long)]
    pub fields_non_ordered_last: bool,

    /// Fields never shown as columns (repeatable)
    #[arg(long, value_name = "FIELDS")]
    pub exclude_fields: Vec<String>,

    /// Use every discovered field as a column
    #[arg(long)]
    pub all_fields: bool,

    /// Output rendering
    #[arg(long, value_enum, default_value = "auto")]
    pub render: RenderChoice,

    /// Free-text patterns, all must match (repeatable)
    #[arg(short = 's', long = "search", value_name = "PATTERN")]
    pub search: Vec<String>,

    /// Limit search to these fields (comma separated)
    #[arg(long, value_name = "FIELDS")]
    pub search_fields: Vec<String>,

    /// Treat search patterns as regular expressions
    #[arg(long)]
    pub search_regex: bool,

    /// Match search and filter patterns case sensitively
    #[arg(long)]
    pub case_sensitive: bool,

    /// Filter expression: `field=pattern`, `field!^`, `not a and (b or c)` (repeatable)
    #[arg(long = "filter", value_name = "EXPR")]
    pub filters: Vec<String>,

    /// Field value equals a glob (repeatable, `FIELD PATTERN` or `FIELD=PATTERN`)
    #[arg(long = "filter-eq", value_name = "FIELD/PATTERN", num_args = 1..=2, allow_hyphen_values = true)]
    pub filter_eq: Vec<String>,

    /// Field value does not equal a glob (repeatable)
    #[arg(long = "filter-neq", value_name = "FIELD/PATTERN", num_args = 1..=2, allow_hyphen_values = true)]
    pub filter_neq: Vec<String>,

    /// Field must exist (repeatable)
    #[arg(long = "filter-exists", value_name = "FIELD")]
    pub filter_exists: Vec<String>,

    /// Field must not exist (repeatable)
    #[arg(long = "filter-not-exists", value_name = "FIELD")]
    pub filter_not_exists: Vec<String>,

    /// Match glob patterns anywhere inside the value
    #[arg(long)]
    pub glob_anywhere: bool,

    /// Treat filter patterns as regular expressions
    #[arg(long)]
    pub filter_regex: bool,

    /// Sort by a field, repeatable, `+` ascending (default) or `-` descending prefix
    #[arg(long = "sort", value_name = "FIELD", allow_hyphen_values = true)]
    pub sort: Vec<String>,

    /// Keep the file order (no sorting)
    #[arg(long)]
    pub no_sort: bool,

    /// Print at most this many records
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Skip this many records before printing
    #[arg(long, value_name = "N", default_value = "0")]
    pub offset: usize,

    /// Print only the last N matching records
    #[arg(long, value_name = "N")]
    pub tail: Option<usize>,

    /// Load at most this many records into memory
    #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_RECORDS)]
    pub max_records: usize,

    /// Output width in columns, 0 disables wrapping and padding
    #[arg(long, value_name = "N")]
    pub width: Option<usize>,

    /// Do not print the column header row
    #[arg(long)]
    pub no_headers: bool,

    /// Truncate cell text to this many characters, 0 disables
    #[arg(long, value_name = "N", default_value_t = 120)]
    pub truncate_values: usize,

    /// Timestamp rendering
    #[arg(long, value_name = "FMT", default_value = "utc")]
    pub time_format: String,

    /// Field path providing the timestamp (repeatable, highest priority first)
    #[arg(long, value_name = "FIELD")]
    pub timestamp_field: Vec<String>,

    /// Field path providing the severity (repeatable)
    #[arg(long, value_name = "FIELD")]
    pub level_field: Vec<String>,

    /// Field path providing the message (repeatable)
    #[arg(long, value_name = "FIELD")]
    pub message_field: Vec<String>,

    /// Segment separator of the plain format
    #[arg(long, value_name = "SEP", default_value = " | ")]
    pub plain_separator: String,

    /// Drop lines that fail to parse instead of keeping them as pseudo records
    #[arg(long)]
    pub no_keep_bad_lines: bool,

    /// Fail on the first line that cannot be parsed
    #[arg(long)]
    pub strict: bool,

    /// Colorize output
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorChoice,

    /// Never colorize output
    #[arg(long)]
    pub no_color: bool,

    /// Mouse capture in interactive mode: auto disables it on ConEmu-class consoles that leak mouse bytes
    #[arg(long, value_enum, default_value = "auto")]
    pub mouse: MouseMode,

    /// Print parse and record counts to stderr
    #[arg(long)]
    pub stats: bool,

    /// Print more detail about what is happening
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Only report errors
    #[arg(short = 'q', long)]
    pub quiet: bool,
}

/// Validated configuration of a run.
#[derive(Clone, Debug)]
pub struct Config {
    pub mode: RunMode,
    pub files: Vec<PathBuf>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub recursive: bool,
    pub format: FormatSelection,
    pub render: RenderChoice,
    pub columns: ColumnSpec,
    pub filters: Vec<Predicate>,
    pub search: SearchSpec,
    pub sorts: Vec<SortSpec>,
    pub limit: Option<usize>,
    pub offset: usize,
    pub tail: Option<usize>,
    pub max_records: usize,
    pub width: usize,
    pub headers: bool,
    pub truncate_values: usize,
    pub time_format: TimeFormat,
    pub color: ColorChoice,
    pub color_enabled: bool,
    pub mouse: MouseMode,
    pub stats: bool,
    pub verbose: u8,
    pub quiet: bool,
    pub keep_bad_lines: bool,
    pub strict: bool,
    pub glob_anywhere: bool,
    pub filter_regex: bool,
    pub plain_separator: String,
    pub aliases: FieldAliases,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: RunMode::Print,
            files: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            recursive: false,
            format: FormatSelection::Auto,
            render: RenderChoice::Auto,
            columns: ColumnSpec::default(),
            filters: Vec::new(),
            search: SearchSpec::default(),
            sorts: Vec::new(),
            limit: None,
            offset: 0,
            tail: None,
            max_records: DEFAULT_MAX_RECORDS,
            width: 0,
            headers: true,
            truncate_values: 120,
            time_format: TimeFormat::UtcDateTime,
            color: ColorChoice::Auto,
            color_enabled: false,
            mouse: MouseMode::Auto,
            stats: false,
            verbose: 0,
            quiet: false,
            keep_bad_lines: true,
            strict: false,
            glob_anywhere: false,
            filter_regex: false,
            plain_separator: " | ".to_owned(),
            aliases: FieldAliases::default(),
        }
    }
}

impl Config {
    /// Test helper: a plain, fully defaulted configuration.
    pub fn stub() -> Self {
        Self::default()
    }

    pub fn from_cli(cli: &Cli) -> Result<Self> {
        let case_insensitive = !cli.case_sensitive;
        let ctx = FilterContext {
            case_insensitive,
            glob_anywhere: cli.glob_anywhere,
            regex: cli.filter_regex,
        };

        let filters = build_filters(cli, &ctx)?;
        let search = build_search(cli, case_insensitive)?;
        let columns = build_columns(cli)?;
        let sorts = build_sorts(cli)?;
        let aliases = build_aliases(cli)?;
        let time_format = TimeFormat::from_str(&cli.time_format).map_err(|_| SilvError::TimeFormat {
            input: cli.time_format.clone(),
        })?;

        let color = if cli.no_color { ColorChoice::Never } else { cli.color };
        let color_enabled = resolve_color(color);

        let config = Self {
            mode: resolve_mode(cli),
            files: cli.files.clone(),
            include: cli.include.clone(),
            exclude: cli.exclude.clone(),
            recursive: cli.recursive,
            format: match cli.format {
                FormatArg::Auto => FormatSelection::Auto,
                FormatArg::Jsonl => FormatSelection::Forced(LogFormat::Jsonl),
                FormatArg::Plain => FormatSelection::Forced(LogFormat::Plain),
            },
            render: cli.render,
            columns,
            filters,
            search,
            sorts,
            limit: cli.limit,
            offset: cli.offset,
            tail: cli.tail,
            max_records: cli.max_records.max(1),
            width: cli.width.unwrap_or_else(terminal_width),
            headers: !cli.no_headers,
            truncate_values: cli.truncate_values,
            time_format,
            color,
            color_enabled,
            mouse: cli.mouse,
            stats: cli.stats,
            verbose: cli.verbose,
            quiet: cli.quiet,
            keep_bad_lines: !cli.no_keep_bad_lines,
            strict: cli.strict,
            glob_anywhere: cli.glob_anywhere,
            filter_regex: cli.filter_regex,
            plain_separator: cli.plain_separator.clone(),
            aliases,
        };
        Ok(config)
    }

    /// Rejects configurations that cannot run (e.g. interactive mode without terminals).
    pub fn validate(&mut self) -> Result<()> {
        if self.mode == RunMode::Interactive && !both_terminals() {
            return Err(SilvError::NotATerminal.into());
        }
        Ok(())
    }

    pub const fn aliases(&self) -> &FieldAliases {
        &self.aliases
    }

    pub fn plain_options(&self) -> PlainOptions {
        PlainOptions {
            separator: self.plain_separator.clone(),
        }
    }

    pub fn filter_context(&self) -> FilterContext {
        FilterContext {
            case_insensitive: self.search.case_insensitive,
            glob_anywhere: self.glob_anywhere,
            regex: self.filter_regex,
        }
    }

    pub fn sources_display(&self) -> String {
        if self.files.is_empty() {
            return "stdin".to_owned();
        }
        self.files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn resolve_mode(cli: &Cli) -> RunMode {
    if cli.interactive {
        return RunMode::Interactive;
    }
    if cli.print {
        return RunMode::Print;
    }
    if both_terminals() {
        RunMode::Interactive
    } else {
        RunMode::Print
    }
}

fn both_terminals() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn resolve_color(choice: ColorChoice) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
                && std::io::stdout().is_terminal()
        },
    }
}

fn terminal_width() -> usize {
    ratatui::crossterm::terminal::size()
        .map(|(width, _)| usize::from(width))
        .unwrap_or(120)
}

fn split_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_paths(values: &[String]) -> Result<Vec<FieldPath>> {
    split_list(values)
        .into_iter()
        .map(|raw| FieldPath::parse(&raw).map_err(|reason| SilvError::FieldPath { input: raw, reason }))
        .collect::<std::result::Result<Vec<_>, SilvError>>()
        .map_err(miette::Report::new)
}

fn build_columns(cli: &Cli) -> Result<ColumnSpec> {
    Ok(ColumnSpec {
        ordered: parse_paths(&cli.fields_order)?,
        rest_last: cli.fields_non_ordered_last,
        excluded: parse_paths(&cli.exclude_fields)?,
        all: cli.all_fields,
    })
}

fn build_aliases(cli: &Cli) -> Result<FieldAliases> {
    FieldAliases::with_defaults(
        &split_list(&cli.timestamp_field),
        &split_list(&cli.level_field),
        &split_list(&cli.message_field),
    )
    .map_err(|(input, reason)| SilvError::FieldPath { input, reason }.into())
}

fn build_sorts(cli: &Cli) -> Result<Vec<SortSpec>> {
    if cli.no_sort {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(cli.sort.len());
    for raw in split_list(&cli.sort) {
        let (dir, name) = match raw.strip_prefix('-') {
            Some(rest) => (SortDir::Desc, rest.to_owned()),
            None => (SortDir::Asc, raw.trim_start_matches('+').to_owned()),
        };
        let field = FieldPath::parse(&name).map_err(|reason| SilvError::FieldPath { input: name, reason })?;
        out.push(SortSpec { field, dir });
    }
    Ok(out)
}

fn build_search(cli: &Cli, case_insensitive: bool) -> Result<SearchSpec> {
    let scope = if cli.search_fields.is_empty() {
        SearchScope::All
    } else {
        SearchScope::Fields(parse_paths(&cli.search_fields)?)
    };
    let patterns = cli
        .search
        .iter()
        .flat_map(|pattern| pattern.split_whitespace().map(str::to_owned))
        .collect();
    Ok(SearchSpec {
        patterns,
        scope,
        regex: cli.search_regex,
        case_insensitive,
    })
}

fn build_filters(cli: &Cli, ctx: &FilterContext) -> Result<Vec<Predicate>> {
    let mut out = Vec::new();
    out.extend(pair_leaves(&cli.filter_eq, LeafOp::Eq, ctx)?);
    out.extend(pair_leaves(&cli.filter_neq, LeafOp::Neq, ctx)?);
    for field in &cli.filter_exists {
        for field in split_list(std::slice::from_ref(field)) {
            out.push(build_leaf(&field, LeafOp::Exists, None, ctx)?);
        }
    }
    for field in &cli.filter_not_exists {
        for field in split_list(std::slice::from_ref(field)) {
            out.push(build_leaf(&field, LeafOp::NotExists, None, ctx)?);
        }
    }
    for expression in &cli.filters {
        out.push(parse_expression(expression, ctx)?);
    }
    Ok(out)
}

/// Accepts both `--filter-eq field pattern` and `--filter-eq field=pattern` in one flat value list.
fn pair_leaves(values: &[String], op: LeafOp, ctx: &FilterContext) -> Result<Vec<Predicate>> {
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < values.len() {
        let first = values[index].as_str();
        if let Some((field, pattern)) = first.split_once('=') {
            out.push(build_leaf(field, op, Some(pattern), ctx)?);
            index += 1;
        } else if index + 1 < values.len() {
            out.push(build_leaf(first, op, Some(&values[index + 1]), ctx)?);
            index += 2;
        } else {
            return Err(SilvError::Argument {
                flag: op.token().to_owned(),
                reason: format!("`{first}` needs a pattern: pass `FIELD PATTERN` or `FIELD=PATTERN`"),
            }
            .into());
        }
    }
    Ok(out)
}

/// Interactive key help, shown by the help overlay.
pub const fn key_help() -> &'static str {
    KEYS
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Config {
        let cli = Cli::try_parse_from(std::iter::once("silv").chain(args.iter().copied())).expect("parses");
        Config::from_cli(&cli).expect("valid config")
    }

    #[test]
    fn defaults_are_stream_when_not_a_terminal() {
        let config = parse(&["app.jsonl"]);
        assert_eq!(config.max_records, DEFAULT_MAX_RECORDS);
        assert!(config.keep_bad_lines);
        assert_eq!(config.time_format, TimeFormat::UtcDateTime);
        assert!(config.headers);
    }

    #[test]
    fn filter_eq_accepts_both_forms() {
        let config = parse(&[
            "-p",
            "--filter-eq",
            "message.logger",
            "aiokafka.*",
            "--filter-neq",
            "level=WARN",
        ]);
        assert_eq!(config.filters.len(), 2);
        assert_eq!(config.filters[0].to_string(), "message.logger=aiokafka.*");
        assert_eq!(config.filters[1].to_string(), "level!=WARN");
    }

    #[test]
    fn exists_filters_and_dsl() {
        let config = parse(&[
            "-p",
            "--filter-exists",
            "trace.id",
            "--filter-not-exists",
            "http.status_code",
            "--filter",
            "level=WARN and msg~disk",
        ]);
        assert_eq!(
            config.filters.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["trace.id^", "http.status_code!^", "level=WARN and msg~disk"]
        );
    }

    #[test]
    fn field_order_and_rest_last() {
        let config = parse(&[
            "-p",
            "-f",
            "@timestamp,level",
            "-f",
            "message",
            "--fields-non-ordered-last",
        ]);
        assert_eq!(
            config
                .columns
                .ordered
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["@timestamp", "level", "message"]
        );
        assert!(config.columns.rest_last);
    }

    #[test]
    fn search_scopes_and_flags() {
        let config = parse(&[
            "-p",
            "-s",
            "disk full",
            "--search-fields",
            "message,msg",
            "--case-sensitive",
        ]);
        assert_eq!(config.search.patterns, vec!["disk", "full"]);
        assert!(!config.search.case_insensitive);
        assert!(matches!(config.search.scope, SearchScope::Fields(ref paths) if paths.len() == 2));
    }

    #[test]
    fn sort_directions() {
        let config = parse(&["-p", "--sort", "+level", "--sort", "-@timestamp"]);
        assert_eq!(config.sorts.len(), 2);
        assert!(matches!(config.sorts[0].dir, SortDir::Asc));
        assert!(matches!(config.sorts[1].dir, SortDir::Desc));
        assert_eq!(config.sorts[1].field.to_string(), "@timestamp");
    }

    #[test]
    fn bad_time_format_and_field_path_are_diagnostics() {
        let cli = Cli::try_parse_from(["silv", "-p", "--time-format", "nope"]).expect("parses");
        let error = Config::from_cli(&cli).expect_err("invalid time format");
        assert!(format!("{error}").contains("nope"));

        let cli = Cli::try_parse_from(["silv", "-p", "-f", "a..b"]).expect("parses");
        let error = Config::from_cli(&cli).expect_err("invalid path");
        assert!(format!("{error}").contains("a..b"), "{error}");
    }

    #[test]
    fn mode_is_explicitly_overridable() {
        let cli = Cli::try_parse_from(["silv", "-p", "app.jsonl"]).expect("parses");
        assert_eq!(Config::from_cli(&cli).expect("config").mode, RunMode::Print);
        let cli = Cli::try_parse_from(["silv", "--print", "app.jsonl"]).expect("parses");
        assert_eq!(Config::from_cli(&cli).expect("config").mode, RunMode::Print);
    }

    #[test]
    fn key_help_lists_the_interactive_keys() {
        assert!(key_help().contains("focus search bar"));
    }
}

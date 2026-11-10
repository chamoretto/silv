//! Stream (grep-like) output: colored records, fields table on top, broken-pipe safe.

use std::io::BufWriter;
use std::io::Write;

use miette::Result;

use crate::cli::Config;
use crate::cli::RenderChoice;
use crate::data::LogStore;
use crate::data::ValueTypes;
use crate::data::view::build_view;
use crate::render::color::Sg;
use crate::render::color::write_styled;
use crate::render::fields;
use crate::render::fields::Column;
use crate::render::record_render::Cell;
use crate::render::record_render::highlight;
use crate::render::record_render::render_record;
use crate::render::record_render::resolve_render;
use crate::render::record_render::table_cells;
use crate::render::table;

/// Rows sampled to size the columns, keeps streaming memory bounded.
const WIDTH_SAMPLE: usize = 500;
const WIDTH_CAP: usize = 60;

/// Derived rendering inputs shared by every stream write path.
struct StreamContext<'a> {
    columns: &'a [Column],
    mode: RenderChoice,
    regexes: &'a [regex::Regex],
    source_labels: &'a [&'a str],
}

/// Writes every matching record and returns how many were written.
pub fn emit(store: &LogStore, config: &Config) -> Result<usize> {
    let columns = fields::resolve_columns(config, store);
    let mode = resolve_render(config, store.format(), !columns.is_empty());
    let search = if config.search.is_empty() {
        None
    } else {
        Some(config.search.compile()?)
    };
    let view =
        build_view(store, &config.filters, search.as_ref(), &config.sorts, config.offset, config.limit, config.tail);
    let regexes = config.search.highlight_regexes();
    let source_labels: Vec<&str> = store.sources().iter().map(|source| source.label.as_str()).collect();
    let ctx = StreamContext {
        columns: &columns,
        mode,
        regexes: &regexes,
        source_labels: &source_labels,
    };

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let written = write_records(&mut out, store, config, &view, &ctx)?;
    out.flush().ok();
    Ok(written)
}

fn write_records(
    out: &mut BufWriter<std::io::StdoutLock<'_>>,
    store: &LogStore,
    config: &Config,
    view: &crate::data::view::View,
    ctx: &StreamContext<'_>,
) -> Result<usize> {
    if ctx.mode == RenderChoice::Table && !ctx.columns.is_empty() {
        write_table(out, store, config, view, ctx)
    } else {
        write_plain_records(out, store, config, view, ctx)
    }
}

fn write_table(
    out: &mut BufWriter<std::io::StdoutLock<'_>>,
    store: &LogStore,
    config: &Config,
    view: &crate::data::view::View,
    ctx: &StreamContext<'_>,
) -> Result<usize> {
    let headers = header_labels(ctx.columns, config);
    let styles = header_styles(ctx.columns);
    let sample: Vec<Vec<Cell>> = view
        .rows
        .iter()
        .take(WIDTH_SAMPLE)
        .map(|index| {
            let record = store.record(usize::try_from(*index).unwrap_or(usize::MAX));
            record.map(|record| table_cells(record, ctx.columns, label_for(record, ctx.source_labels), config))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();

    let mut widths = table::natural_widths(&headers, &sample, WIDTH_CAP);
    table::fit_widths(&mut widths, config.width);
    let numeric = numeric_columns(ctx.columns, store);

    let mut written = 0usize;
    let mut failure: Option<std::io::Error> = None;

    if config.headers {
        let header_cells: Vec<Cell> = headers
            .iter()
            .zip(&styles)
            .map(|(text, style)| Cell::new(text.clone(), *style))
            .collect();
        if let Err(error) = table::write_row(out, &header_cells, &widths, config.color_enabled, &numeric, "  ") {
            failure = Some(error);
        }
        if failure.is_none() {
            writeln!(out).ok();
        }
    }

    if failure.is_none() {
        for index in &view.rows {
            let Some(record) = store.record(usize::try_from(*index).unwrap_or(usize::MAX)) else {
                continue;
            };
            let cells =
                highlight(table_cells(record, ctx.columns, label_for(record, ctx.source_labels), config), ctx.regexes);
            match table::write_row(out, &cells, &widths, config.color_enabled, &numeric, "  ") {
                Ok(()) => {
                    writeln!(out).ok();
                    written += 1;
                },
                Err(error) => {
                    failure = Some(error);
                    break;
                },
            }
        }
    }

    match failure {
        Some(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(written),
        Some(error) => Err(crate::error::SilvError::OutputWrite { cause: error }.into()),
        None => Ok(written),
    }
}

fn write_plain_records(
    out: &mut BufWriter<std::io::StdoutLock<'_>>,
    store: &LogStore,
    config: &Config,
    view: &crate::data::view::View,
    ctx: &StreamContext<'_>,
) -> Result<usize> {
    let mut written = 0usize;
    let mut failure: Option<std::io::Error> = None;
    for index in &view.rows {
        let Some(record) = store.record(usize::try_from(*index).unwrap_or(usize::MAX)) else {
            continue;
        };
        let cells = render_record(record, ctx.mode, ctx.columns, label_for(record, ctx.source_labels), config);
        let cells = if ctx.mode == RenderChoice::Json || ctx.mode == RenderChoice::Jsonl {
            cells
        } else {
            highlight(cells, ctx.regexes)
        };
        match write_cells(out, config.color_enabled, &cells) {
            Ok(()) => {
                writeln!(out).ok();
                written += 1;
            },
            Err(error) => {
                failure = Some(error);
                break;
            },
        }
    }
    match failure {
        Some(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(written),
        Some(error) => Err(crate::error::SilvError::OutputWrite { cause: error }.into()),
        None => Ok(written),
    }
}

fn write_cells(out: &mut impl Write, enabled: bool, cells: &[Cell]) -> std::io::Result<()> {
    for cell in cells {
        write_styled(out, enabled, cell.style, &cell.text)?;
    }
    Ok(())
}

fn header_labels(columns: &[Column], config: &Config) -> Vec<String> {
    columns
        .iter()
        .map(|column| {
            let marker = config
                .sorts
                .iter()
                .find(|spec| Column::from_path(&spec.field) == *column)
                .map(|spec| format!(" {}", spec.dir.marker()))
                .unwrap_or_default();
            format!("{}{marker}", column.label())
        })
        .collect()
}

fn header_styles(columns: &[Column]) -> Vec<Sg> {
    columns.iter().map(|_| Sg::Heading).collect()
}

fn numeric_columns(columns: &[Column], store: &LogStore) -> Vec<bool> {
    columns
        .iter()
        .map(|column| match column {
            Column::Line => true,
            Column::Field(path) => store.field_by_path(path).is_some_and(|info| {
                info.types.contains(ValueTypes::NUMBER) && !info.types.contains(ValueTypes::STRING)
            }),
            _ => false,
        })
        .collect()
}

fn label_for<'a>(record: &crate::model::record::LogRecord, source_labels: &[&'a str]) -> &'a str {
    source_labels
        .get(usize::try_from(record.source).unwrap_or(usize::MAX))
        .copied()
        .unwrap_or("<unknown>")
}

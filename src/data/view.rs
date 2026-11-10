//! Row selection: filtering, sorting and windowing shared by both modes.

use rayon::prelude::*;

use crate::cli::SortSpec;
use crate::data::LogStore;
use crate::filter::Predicate;
use crate::render::fields::sort_indices;

/// Sets the parallel threshold above which filtering uses all cores.
pub const PARALLEL_THRESHOLD: usize = 20_000;

/// A materialized view of the store.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub rows: Vec<u32>,
    pub matched: usize,
}

impl View {
    pub const fn len(&self) -> usize {
        self.rows.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn record<'a>(&self, store: &'a LogStore, position: usize) -> Option<&'a crate::model::record::LogRecord> {
        self.rows
            .get(position)
            .and_then(|index| store.record(usize::try_from(*index).unwrap_or(usize::MAX)))
    }
}

/// Applies filters, then sorting, then the output window.
pub fn build_view(
    store: &LogStore,
    filters: &[Predicate],
    search: Option<&Predicate>,
    sorts: &[SortSpec],
    offset: usize,
    limit: Option<usize>,
    tail: Option<usize>,
) -> View {
    let predicate = combine(filters, search);
    let rows = filter_rows(store, &predicate);
    let mut rows = rows;
    sort_indices(store, &mut rows, sorts);
    let matched = rows.len();

    if offset > 0 {
        let drop = offset.min(rows.len());
        rows.drain(..drop);
    }
    if let Some(tail) = tail
        && tail < rows.len()
    {
        let start = rows.len() - tail;
        rows.drain(..start);
    }
    if let Some(limit) = limit
        && limit < rows.len()
    {
        rows.truncate(limit);
    }
    View { rows, matched }
}

fn combine(filters: &[Predicate], search: Option<&Predicate>) -> Predicate {
    let mut children: Vec<Predicate> = filters.to_vec();
    if let Some(search) = search {
        children.push(search.clone());
    }
    Predicate::all(children)
}

fn filter_rows(store: &LogStore, predicate: &Predicate) -> Vec<u32> {
    if store.len() <= PARALLEL_THRESHOLD {
        (0..u32::try_from(store.len()).unwrap_or(u32::MAX))
            .filter(|index| {
                store
                    .record(usize::try_from(*index).unwrap_or(usize::MAX))
                    .is_some_and(|record| predicate.eval(record))
            })
            .collect()
    } else {
        (0..u32::try_from(store.len()).unwrap_or(u32::MAX))
            .into_par_iter()
            .filter(|index| {
                store
                    .record(usize::try_from(*index).unwrap_or(usize::MAX))
                    .is_some_and(|record| predicate.eval(record))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Config;
    use crate::cli::SortDir;
    use crate::filter::MatchKind;
    use crate::filter::Matcher;
    use crate::filter::SearchSpec;
    use crate::model::field::FieldPath;

    fn store_of(records: &[&str]) -> LogStore {
        let config = Config::stub();
        let bundle = crate::parse::input::InputBundle {
            sources: vec![crate::parse::input::RawSource::for_test(
                "t.jsonl",
                records.join("\n").into_bytes(),
            )],
            read_stdin: false,
        };
        LogStore::build(&bundle, &config).expect("builds")
    }

    #[test]
    fn filters_and_windows_compose() {
        let store = store_of(&["{\"a\":1}", "{\"a\":2}", "{\"a\":3}", "{\"b\":1}"]);
        let filter = crate::filter::build_leaf(
            "a",
            crate::filter::LeafOp::Exists,
            None,
            &crate::filter::dsl::FilterContext::default(),
        )
        .expect("leaf");
        let view = build_view(&store, &[filter], None, &[], 1, Some(2), None);
        assert_eq!(view.rows, vec![1, 2]);
        assert_eq!(view.matched, 3);
    }

    #[test]
    fn tail_keeps_the_last_rows() {
        let store = store_of(&["{\"a\":1}", "{\"a\":2}", "{\"a\":3}"]);
        let view = build_view(&store, &[], None, &[], 0, None, Some(2));
        assert_eq!(view.rows, vec![1, 2]);
    }

    #[test]
    fn search_narrows_the_view() {
        let store = store_of(&["{\"message\":\"disk full\"}", "{\"message\":\"ok\"}"]);
        let spec = SearchSpec {
            patterns: vec!["disk".to_owned()],
            ..SearchSpec::default()
        };
        let predicate = spec.compile().expect("compiles");
        let view = build_view(&store, &[], Some(&predicate), &[], 0, None, None);
        assert_eq!(view.rows, vec![0]);
    }

    #[test]
    fn sorted_view_uses_columns() {
        let store = store_of(&["{\"a\":3}", "{\"a\":1}", "{\"a\":2}"]);
        let sorts = vec![SortSpec {
            field: FieldPath::parse("a").unwrap(),
            dir: SortDir::Desc,
        }];
        let view = build_view(&store, &[], None, &sorts, 0, Some(2), None);
        assert_eq!(view.rows, vec![0, 2]);
    }

    #[test]
    fn view_record_lookup_follows_rows() {
        let store = store_of(&["{\"a\":1}"]);
        let view = build_view(&store, &[], None, &[], 0, None, None);
        assert!(view.record(&store, 0).is_some());
        assert!(view.record(&store, 5).is_none());
    }

    #[test]
    fn glob_filter_leaves_the_pattern_kind_visible() {
        let matcher = Matcher::compile("a*", MatchKind::Glob, true, false).expect("compiles");
        assert_eq!(matcher.kind(), MatchKind::Glob);
    }
}

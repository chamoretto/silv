//! Filtering, search and the shared predicate tree.

pub mod dsl;
pub mod predicate;
pub mod search;

use miette::Result;

pub use predicate::LeafOp;
pub use predicate::MatchKind;
pub use predicate::Matcher;
pub use predicate::Predicate;
pub use search::SearchScope;
pub use search::SearchSpec;

use crate::model::field::FieldPath;

/// Builds a single filter leaf from a field name and optional pattern.
pub fn build_leaf(field: &str, op: LeafOp, pattern: Option<&str>, ctx: &dsl::FilterContext) -> Result<Predicate> {
    let path = FieldPath::parse(field).map_err(|reason| crate::error::SilvError::FieldPath {
        input: field.to_owned(),
        reason,
    })?;
    let matcher = if op.takes_pattern() {
        let pattern = pattern.ok_or_else(|| crate::error::SilvError::Argument {
            flag: op.to_string(),
            reason: "this filter operator needs a pattern".to_owned(),
        })?;
        Some(Matcher::compile(pattern, ctx.kind_for(op), ctx.case_insensitive, ctx.glob_anywhere)?)
    } else {
        None
    };
    Ok(Predicate::leaf(path, op, matcher))
}

/// Parses the `field=pattern` style accepted by `--filter`.
pub fn parse_expression(input: &str, ctx: &dsl::FilterContext) -> Result<Predicate> {
    dsl::parse(input, ctx).map_err(|reason| {
        miette::Report::new(crate::error::SilvError::FilterParse {
            input: input.to_owned(),
            reason,
        })
    })
}

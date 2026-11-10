//! Filter mini-DSL: `field=pattern`, `field!^`, `not a and (b or c)`.

use crate::filter::predicate::LeafOp;
use crate::filter::predicate::MatchKind;
use crate::filter::predicate::Matcher;
use crate::filter::predicate::Predicate;
use crate::model::field::FieldPath;

/// Knobs that decide how patterns in the DSL are compiled.
#[derive(Clone, Copy, Debug)]
pub struct FilterContext {
    pub case_insensitive: bool,
    pub glob_anywhere: bool,
    pub regex: bool,
}

impl Default for FilterContext {
    fn default() -> Self {
        Self {
            case_insensitive: true,
            glob_anywhere: false,
            regex: false,
        }
    }
}

impl FilterContext {
    /// Chooses the matching style for an operator.
    pub const fn kind_for(self, op: LeafOp) -> MatchKind {
        if self.regex {
            return MatchKind::Regex;
        }
        match op {
            LeafOp::Contains | LeafOp::NotContains => MatchKind::Contains,
            _ => MatchKind::Glob,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    Not,
    And,
    Or,
    Leaf {
        field: String,
        op: LeafOp,
        value: Option<String>,
    },
}

/// Parses one filter expression.
pub fn parse(input: &str, ctx: &FilterContext) -> Result<Predicate, String> {
    let raw = raw_tokens(input)?;
    let tokens = classify(&raw)?;
    if tokens.is_empty() {
        return Err("empty expression".to_owned());
    }
    let mut position = 0usize;
    let predicate = parse_or(&tokens, &mut position, ctx)?;
    if position != tokens.len() {
        return Err(format!("unexpected `{}`", describe(&tokens[position])));
    }
    Ok(predicate)
}

fn describe(token: &Token) -> String {
    match token {
        Token::LParen => "(".to_owned(),
        Token::RParen => ")".to_owned(),
        Token::Not => "not".to_owned(),
        Token::And => "and".to_owned(),
        Token::Or => "or".to_owned(),
        Token::Leaf { field, op, .. } => format!("{field}{op}"),
    }
}

fn raw_tokens(input: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '"' => {
                quoted = !quoted;
                current.push('"');
                index += 1;
            },
            '\\' if index + 1 < chars.len() => {
                current.push(chars[index + 1]);
                index += 2;
            },
            _ if !quoted && ch.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                index += 1;
            },
            '(' | ')' if !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                out.push(ch.to_string());
                index += 1;
            },
            _ => {
                current.push(ch);
                index += 1;
            },
        }
    }
    if quoted {
        return Err("unterminated quoted section".to_owned());
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}

fn classify(raw: &[String]) -> Result<Vec<Token>, String> {
    let mut out = Vec::with_capacity(raw.len());
    for token in raw {
        match token.as_str() {
            "(" => out.push(Token::LParen),
            ")" => out.push(Token::RParen),
            "not" | "!" => out.push(Token::Not),
            "and" | "&&" => out.push(Token::And),
            "or" | "||" => out.push(Token::Or),
            _ => out.push(classify_leaf(token)?),
        }
    }
    Ok(out)
}

fn classify_leaf(token: &str) -> Result<Token, String> {
    if let Some(rest) = token.strip_prefix('"') {
        let close = rest.find('"').ok_or("unterminated quoted field")?;
        let split = close + 2;
        return leaf_after_field(&token[..split], &token[split..]);
    }
    if let Some(field) = token.strip_suffix("!^") {
        return leaf_with(field, LeafOp::NotExists, None);
    }
    if let Some(field) = token.strip_suffix('^') {
        return leaf_with(field, LeafOp::Exists, None);
    }
    for (needle, op) in [
        ("!=", LeafOp::Neq),
        ("!~", LeafOp::NotContains),
        ("~", LeafOp::Contains),
        ("=", LeafOp::Eq),
    ] {
        if let Some(index) = token.find(needle) {
            let field = &token[..index];
            let value = unquote(&token[index + needle.len()..]);
            if value.is_empty() {
                return Err(format!("operator `{needle}` needs a pattern"));
            }
            return leaf_with(field, op, Some(value));
        }
    }
    Err(format!("`{token}` has no operator, expected `field=pattern`, `field~text`, `field^` or `field!^`"))
}

fn leaf_after_field(field: &str, rest: &str) -> Result<Token, String> {
    let (op, value) = if let Some(v) = rest.strip_prefix("!^") {
        if !v.is_empty() {
            return Err(format!("text after `!^`: `{v}`"));
        }
        (LeafOp::NotExists, None)
    } else if let Some(v) = rest.strip_prefix('^') {
        if !v.is_empty() {
            return Err(format!("text after `^`: `{v}`"));
        }
        (LeafOp::Exists, None)
    } else if let Some(v) = rest.strip_prefix("!=") {
        (LeafOp::Neq, Some(unquote(v)))
    } else if let Some(v) = rest.strip_prefix("!~") {
        (LeafOp::NotContains, Some(unquote(v)))
    } else if let Some(v) = rest.strip_prefix('~') {
        (LeafOp::Contains, Some(unquote(v)))
    } else if let Some(v) = rest.strip_prefix('=') {
        (LeafOp::Eq, Some(unquote(v)))
    } else {
        return Err(format!("`{rest}` is not an operator"));
    };
    if value.as_deref().is_some_and(str::is_empty) {
        return Err(format!("operator `{}` needs a pattern", op.token()));
    }
    leaf_with(field, op, value)
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn leaf_with(field: &str, op: LeafOp, value: Option<String>) -> Result<Token, String> {
    if field.is_empty() {
        return Err(format!("missing field name before `{}`", op.token()));
    }
    Ok(Token::Leaf {
        field: field.to_owned(),
        op,
        value,
    })
}

fn parse_or(tokens: &[Token], position: &mut usize, ctx: &FilterContext) -> Result<Predicate, String> {
    let mut children = vec![parse_and(tokens, position, ctx)?];
    while *position < tokens.len() && tokens[*position] == Token::Or {
        *position += 1;
        children.push(parse_and(tokens, position, ctx)?);
    }
    Ok(Predicate::any(children))
}

fn parse_and(tokens: &[Token], position: &mut usize, ctx: &FilterContext) -> Result<Predicate, String> {
    let mut children = vec![parse_not(tokens, position, ctx)?];
    while *position < tokens.len() && tokens[*position] == Token::And {
        *position += 1;
        children.push(parse_not(tokens, position, ctx)?);
    }
    Ok(Predicate::all(children))
}

fn parse_not(tokens: &[Token], position: &mut usize, ctx: &FilterContext) -> Result<Predicate, String> {
    if *position < tokens.len() && tokens[*position] == Token::Not {
        *position += 1;
        return Ok(Predicate::Not(Box::new(parse_not(tokens, position, ctx)?)));
    }
    parse_primary(tokens, position, ctx)
}

fn parse_primary(tokens: &[Token], position: &mut usize, ctx: &FilterContext) -> Result<Predicate, String> {
    let Some(token) = tokens.get(*position) else {
        return Err("expression ends with a missing operand".to_owned());
    };
    match token {
        Token::LParen => {
            *position += 1;
            let inner = parse_or(tokens, position, ctx)?;
            if tokens.get(*position) != Some(&Token::RParen) {
                return Err("missing `)`".to_owned());
            }
            *position += 1;
            Ok(inner)
        },
        Token::Leaf { field, op, value } => {
            *position += 1;
            let path = FieldPath::parse(field).map_err(|reason| format!("field `{field}`: {reason}"))?;
            let matcher = if op.takes_pattern() {
                let value = value.clone().unwrap_or_default();
                Some(
                    Matcher::compile(&value, ctx.kind_for(*op), ctx.case_insensitive, ctx.glob_anywhere)
                        .map_err(|error| error.to_string())?,
                )
            } else {
                None
            };
            Ok(Predicate::leaf(path, *op, matcher))
        },
        other => Err(format!("unexpected `{}`", describe(other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dsl(input: &str) -> Result<Predicate, String> {
        parse(input, &FilterContext::default())
    }

    #[test]
    fn parses_simple_leaves() {
        assert_eq!(dsl("level=WARN").map(|p| p.to_string()).ok().as_deref(), Some("level=WARN"));
        assert_eq!(dsl("level!=INFO").map(|p| p.to_string()).ok().as_deref(), Some("level!=INFO"));
        assert_eq!(dsl("trace.id^").map(|p| p.to_string()).ok().as_deref(), Some("trace.id^"));
        assert_eq!(dsl("trace.id!^").map(|p| p.to_string()).ok().as_deref(), Some("trace.id!^"));
        assert_eq!(dsl("msg~timeout").map(|p| p.to_string()).ok().as_deref(), Some("msg~timeout"));
    }

    #[test]
    fn parses_composites_with_precedence() {
        assert_eq!(
            dsl("level=WARN and msg~disk or trace.id^")
                .map(|p| p.to_string())
                .ok()
                .as_deref(),
            Some("(level=WARN and msg~disk) or trace.id^")
        );
        assert_eq!(dsl("not level=INFO").map(|p| p.to_string()).ok().as_deref(), Some("not level=INFO"));
        assert_eq!(dsl("not (a=1 or b=2)").map(|p| p.to_string()).ok().as_deref(), Some("not (a=1 or b=2)"));
    }

    #[test]
    fn quoted_values_keep_spaces() {
        let parsed = dsl("message=\"disk full\"").expect("parses");
        assert_eq!(parsed.to_string(), "message=\"disk full\"");
    }

    #[test]
    fn quoted_fields_keep_dots() {
        let parsed = dsl("\"a.b\"=1").expect("parses");
        assert_eq!(parsed.to_string(), "\"a.b\"=1");
    }

    #[test]
    fn rejects_broken_expressions() {
        assert!(dsl("level").is_err());
        assert!(dsl("level=").is_err());
        assert!(dsl("a=1 and").is_err());
        assert!(dsl("(a=1").is_err());
        assert!(dsl("a=\"1").is_err());
        assert!(dsl("").is_err());
    }

    #[test]
    fn evaluates_parsed_filter() {
        let object = serde_json::from_str(r#"{"message":{"logger":"aiokafka.consumer"},"level":"WARN"}"#)
            .expect("fixture object");
        let record = crate::model::record::LogRecord::from_object(
            0,
            1,
            0,
            object,
            &crate::model::record::FieldAliases::with_defaults(&[], &[], &[]).expect("aliases"),
        );
        let filter = dsl("message.logger=aiokafka.* and level!=INFO").expect("parses");
        assert!(filter.eval(&record));
        let filter = dsl("message.logger=aiokafka.* and level=WARN").expect("parses");
        assert!(filter.eval(&record));
        let filter = dsl("not message.logger=aiokafka.*").expect("parses");
        assert!(!filter.eval(&record));
    }
}

//! Shared test fixtures: value, record, store and alias constructors.

use ijson::IValue;

use crate::cli::Config;
use crate::data::LogStore;
use crate::model::record::FieldAliases;
use crate::model::record::LogRecord;

pub fn ivalue(json: serde_json::Value) -> IValue {
    ijson::to_value(json).expect("fixture value")
}

pub fn map(text: &str) -> IValue {
    serde_json::from_str(text).expect("fixture value")
}

pub fn aliases() -> FieldAliases {
    FieldAliases::with_defaults(&[], &[], &[]).expect("defaults")
}

pub fn record(text: &str) -> LogRecord {
    let object = serde_json::from_str(text).expect("fixture object");
    LogRecord::from_object(0, 1, 0, object, &aliases())
}

pub fn store_of(records: &[&str]) -> LogStore {
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

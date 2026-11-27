//! D17's importers on any text: MongoDB's (JSON lines, arrays, extended JSON, typed CSV headers) and the bulk
//! import's JSON-to-CSV and type guessing. They must refuse what they can't read, never panic.
#![no_main]
use dalan_app_lib::dbs::bulkfiles::{infer, json_to_csv};
use dalan_app_lib::dbs::schemadiff::Family;
use dalan_app_lib::mongo::import::parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = parse(&text);
    if let Ok(csv) = json_to_csv(&text) {
        let rows = dalan_app_lib::sql::transfer::parse_csv(&csv, ',');
        if let Some((head, body)) = rows.split_first() {
            let _ = infer(Family::Pg, head, body);
            let _ = infer(Family::My, head, body);
        }
    }
});

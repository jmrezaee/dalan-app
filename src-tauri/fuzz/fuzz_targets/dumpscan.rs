//! The plain-SQL scanner on any bytes, both clients, preview and rewrite, fed in uneven chunks: it must never panic.
#![no_main]
use dalan_app_lib::dbs::dumpscan::{Content, Dialect, Existing, Rewrite, Scanner};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let split = data.first().map_or(1, |b| usize::from(*b).max(1));
    for d in [Dialect::Pg, Dialect::My] {
        for rw in [None, Some(Rewrite { section: Some("a".into()), skip_owner_acl: true, include_roles: false, existing: Existing::Replace, content: Content::All, strip_definer: true, keep_sandbox_line: true, tables: Some(vec!["t".into()]) })] {
            let mut s = Scanner::new(d, rw, split % 2 == 0);
            let mut out = Vec::new();
            let ok = data.chunks(split).all(|c| s.feed(c, &mut out).is_ok());
            if ok {
                let _ = s.finish(&mut out);
            }
        }
    }
});

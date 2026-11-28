//! Format detection and every reader (pg archive TOC and data, Mongo archive, tar, compression) on any bytes, as a
//! picked file: it must never panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = std::env::temp_dir().join(format!("dalan-fuzz-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("dump");
    if std::fs::write(&p, data).is_err() {
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let _ = rt.block_on(dalan_app_lib::dbs::dumpfile::pick(&p, "f".into()));
});

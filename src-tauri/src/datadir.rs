//! Where Dalan keeps its data: `vault.db`, logs and edit copies.
//!
//! A release build uses the app data folder (`~/Library/Application Support/app.dalan.desktop` on a Mac). Every debug
//! build (`pnpm tauri dev`, `pnpm tauri build --debug`, the test runners) uses a folder of its own next to it,
//! `app.dalan.desktop.dev`, or `DALAN_DATA_DIR` when that is set. A debug build is newer than the released app and
//! migrates the database forward; on 2026-11-28 one did that to the real vault, and the released app (which knows
//! fewer migrations) stopped starting. A debug build never opens the release folder.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::error::{AppError, AppResult};

/// The folder for this build, from the platform's app data folder.
pub fn for_build(release_dir: &Path, debug: bool, env: Option<PathBuf>) -> PathBuf {
    if !debug {
        return release_dir.to_path_buf();
    }
    if let Some(d) = env.filter(|d| !d.as_os_str().is_empty() && d != release_dir) {
        return d;
    }
    let mut name = release_dir
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    // Built with tauri.dev.conf.json the identifier is already `….dev`.
    if name.to_string_lossy().ends_with(".dev") {
        return release_dir.to_path_buf();
    }
    name.push(".dev");
    release_dir.with_file_name(name)
}

/// This build's data folder.
pub fn data_dir<R: Runtime>(app: &AppHandle<R>) -> AppResult<PathBuf> {
    let release = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(for_build(
        &release,
        cfg!(debug_assertions),
        std::env::var_os("DALAN_DATA_DIR").map(PathBuf::from),
    ))
}

#[cfg(test)]
mod tests {
    use super::for_build;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_debug_build_never_uses_the_release_folder() {
        let real = Path::new("/Users/me/Library/Application Support/app.dalan.desktop");
        assert_eq!(for_build(real, false, None), real);
        assert_eq!(
            for_build(real, true, None),
            Path::new("/Users/me/Library/Application Support/app.dalan.desktop.dev")
        );
        assert_eq!(
            for_build(real, true, Some(PathBuf::from("/tmp/dalan-test"))),
            Path::new("/tmp/dalan-test")
        );
        // Pointing DALAN_DATA_DIR at the release folder doesn't reach it either.
        assert_eq!(
            for_build(real, true, Some(real.to_path_buf())),
            Path::new("/Users/me/Library/Application Support/app.dalan.desktop.dev")
        );
        let dev = Path::new("/Users/me/Library/Application Support/app.dalan.desktop.dev");
        assert_eq!(for_build(dev, true, None), dev);
        // A release build ignores DALAN_DATA_DIR.
        assert_eq!(for_build(real, false, Some(PathBuf::from("/tmp/x"))), real);
    }
}

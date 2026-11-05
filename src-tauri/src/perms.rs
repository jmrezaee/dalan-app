//! Owner-only permissions for everything Dalan keeps on disk (Unix; Windows profiles are already per-user).

use std::path::Path;

/// Creates `dir` (and parents) and makes it readable by this user only (0700).
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    set_mode(dir, 0o700)
}

/// Makes an existing file readable and writable by this user only (0600). Missing files are skipped.
pub fn private_file(file: &Path) -> std::io::Result<()> {
    if file.exists() {
        set_mode(file, 0o600)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn dirs_and_files_become_owner_only() {
        let root = std::env::temp_dir().join(format!("dalan-perms-{}", uuid::Uuid::now_v7()));
        let d = root.join("a/b");
        private_dir(&d).unwrap();
        let f = d.join("x");
        std::fs::write(&f, b"1").unwrap();
        private_file(&f).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&d), 0o700);
        assert_eq!(mode(&f), 0o600);
        private_file(&root.join("missing")).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

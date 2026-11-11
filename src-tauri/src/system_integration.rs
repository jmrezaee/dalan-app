//! Windows integration the installer sets up and Settings > System can change later (spec section 9.3): opening
//! `ssh://` links with Dalan, and starting Dalan when the person signs in. The registry keys are exactly the ones the
//! installer writes (src-tauri/windows/options-page.nsh), so the two always agree.
//!
//! The registry code is Windows-only. Everything else (what the keys hold, how a handler is recognised) is plain
//! functions, so it is tested on every platform.

use serde::Serialize;
use specta::Type;

/// What Settings shows. On other systems `supported` is false and the rest is empty.
#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct SystemIntegration {
    pub supported: bool,
    /// Dalan is the `ssh://` handler.
    pub ssh_links: bool,
    /// Dalan starts when the person signs in.
    pub autostart: bool,
    /// Another app opens `ssh://` links (its name), so ticking the switch takes them over.
    pub other_handler: Option<String>,
    /// The person picked that app in Windows Settings: Dalan can't override it, only point them there.
    pub pinned: bool,
    /// `"user"` (installed for this person) or `"allusers"`, from the installer's saved choices.
    pub edition: Option<String>,
    pub install_dir: Option<String>,
}

pub const SSH_PROG_ID: &str = "Dalan.ssh";

/// `"<exe>" --open-url "%1"` : the command for the `ssh` and `Dalan.ssh` keys.
pub fn ssh_command(exe: &str) -> String {
    format!("\"{exe}\" --open-url \"%1\"")
}

/// `"<exe>" --minimized` : the Run value.
pub fn autostart_command(exe: &str) -> String {
    format!("\"{exe}\" --minimized")
}

/// The file name of the program a registry command starts: `"C:\Tools\putty.exe" "%1"` gives `putty.exe`.
pub fn exe_name_of(command: &str) -> String {
    let c = command.trim();
    let path = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => c.split(' ').next().unwrap_or(""),
    };
    path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

/// A command that starts our own program (compared by file name, so a moved install still counts as ours).
pub fn is_our_command(command: &str, our_exe_name: &str) -> bool {
    exe_name_of(command).eq_ignore_ascii_case(our_exe_name)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::error::{AppError, AppResult};
    use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const CAPS: &str = r"Software\Dalan\Capabilities";
    const INSTALLER: &str = r"Software\Dalan\Installer";
    const PRODUCT: &str = "Dalan";

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    fn our_exe() -> AppResult<(String, String)> {
        let exe = std::env::current_exe().map_err(|e| AppError::internal(e.to_string()))?;
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok((exe.display().to_string(), name))
    }

    fn read_str(root: &RegKey, path: &str, name: &str) -> Option<String> {
        root.open_subkey_with_flags(path, KEY_READ)
            .ok()?
            .get_value::<String, _>(name)
            .ok()
    }

    pub fn get() -> AppResult<SystemIntegration> {
        let (exe, name) = our_exe()?;
        let user = hkcu();
        let mut info = SystemIntegration {
            supported: true,
            ..Default::default()
        };
        info.install_dir = std::path::Path::new(&exe)
            .parent()
            .map(|p| p.display().to_string());
        info.autostart = read_str(&user, RUN, PRODUCT).is_some_and(|c| is_our_command(&c, &name));

        // 1. A default picked in Windows Settings is hash-protected: we can only say so.
        let choice = read_str(
            &user,
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\ssh\UserChoice",
            "ProgId",
        );
        if choice.as_deref() == Some(SSH_PROG_ID) {
            info.ssh_links = true;
        } else if let Some(prog) = choice.filter(|p| !p.is_empty()) {
            let friendly =
                read_str(&RegKey::predef(HKEY_CLASSES_ROOT), &prog, "").filter(|f| !f.is_empty());
            info.pinned = true;
            info.other_handler = Some(friendly.unwrap_or(prog));
        } else {
            // 2. Whoever registered the scheme: this user's key first, then the machine's.
            let cmd = read_str(&user, r"Software\Classes\ssh\shell\open\command", "")
                .or_else(|| {
                    read_str(
                        &RegKey::predef(HKEY_CLASSES_ROOT),
                        r"ssh\shell\open\command",
                        "",
                    )
                })
                .filter(|c| !c.is_empty());
            match cmd {
                Some(c) if is_our_command(&c, &name) => info.ssh_links = true,
                Some(c) => info.other_handler = Some(exe_name_of(&c)),
                None => {}
            }
        }
        for root in [&user, &RegKey::predef(HKEY_LOCAL_MACHINE)] {
            if let Some(e) = read_str(root, INSTALLER, "Edition") {
                info.edition = Some(e);
                break;
            }
        }
        Ok(info)
    }

    /// Takes the ssh:// links (and the Default apps entry) for Dalan, or gives them back when they are ours.
    pub fn set_ssh_links(on: bool) -> AppResult<()> {
        let (exe, name) = our_exe()?;
        let user = hkcu();
        let cmd = ssh_command(&exe);
        let map =
            |e: std::io::Error| AppError::internal(format!("Windows refused the change: {e}"));
        if on {
            let (k, _) = user.create_subkey(r"Software\Classes\ssh").map_err(map)?;
            k.set_value("", &"URL:SSH link").map_err(map)?;
            k.set_value("URL Protocol", &"").map_err(map)?;
            user.create_subkey(r"Software\Classes\ssh\DefaultIcon")
                .map_err(map)?
                .0
                .set_value("", &format!("\"{exe}\",0"))
                .map_err(map)?;
            user.create_subkey(r"Software\Classes\ssh\shell\open\command")
                .map_err(map)?
                .0
                .set_value("", &cmd)
                .map_err(map)?;
            user.create_subkey(r"Software\Classes\Dalan.ssh\shell\open\command")
                .map_err(map)?
                .0
                .set_value("", &cmd)
                .map_err(map)?;
            let caps = user.create_subkey(CAPS).map_err(map)?.0;
            caps.set_value("ApplicationName", &PRODUCT).map_err(map)?;
            caps.set_value("ApplicationDescription", &"SSH client")
                .map_err(map)?;
            user.create_subkey(format!(r"{CAPS}\URLAssociations"))
                .map_err(map)?
                .0
                .set_value("ssh", &SSH_PROG_ID)
                .map_err(map)?;
            user.create_subkey(r"Software\RegisteredApplications")
                .map_err(map)?
                .0
                .set_value(PRODUCT, &CAPS)
                .map_err(map)?;
        } else {
            // Only what is ours: another app's handler stays.
            if read_str(&user, r"Software\Classes\ssh\shell\open\command", "")
                .is_some_and(|c| is_our_command(&c, &name))
            {
                let _ = user.delete_subkey_all(r"Software\Classes\ssh");
            }
            if read_str(&user, r"Software\Classes\Dalan.ssh\shell\open\command", "")
                .is_some_and(|c| is_our_command(&c, &name))
            {
                let _ = user.delete_subkey_all(r"Software\Classes\Dalan.ssh");
                let _ = user.delete_subkey_all(CAPS);
                if let Ok(k) = user.open_subkey_with_flags(
                    r"Software\RegisteredApplications",
                    winreg::enums::KEY_WRITE,
                ) {
                    let _ = k.delete_value(PRODUCT);
                }
            }
        }
        remember("SshLinks", on)
    }

    pub fn set_autostart(on: bool) -> AppResult<()> {
        let (exe, _) = our_exe()?;
        let map =
            |e: std::io::Error| AppError::internal(format!("Windows refused the change: {e}"));
        let run = hkcu().create_subkey(RUN).map_err(map)?.0;
        if on {
            run.set_value(PRODUCT, &autostart_command(&exe))
                .map_err(map)?;
        } else {
            match run.delete_value(PRODUCT) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(map(e)),
                _ => {}
            }
        }
        remember("Autostart", on)
    }

    /// The installer pre-fills its page from these values (written to this user's key: for the all-users edition the
    /// machine key needs administrator rights, so a change made here is not picked up by a reinstall).
    fn remember(name: &str, on: bool) -> AppResult<()> {
        let k = hkcu()
            .create_subkey(INSTALLER)
            .map_err(|e| AppError::internal(e.to_string()))?
            .0;
        k.set_value(name, &u32::from(on))
            .map_err(|e| AppError::internal(e.to_string()))
    }
}

#[cfg(windows)]
pub use imp::{get, set_autostart, set_ssh_links};

#[cfg(not(windows))]
pub fn get() -> crate::error::AppResult<SystemIntegration> {
    Ok(SystemIntegration::default())
}

#[cfg(not(windows))]
pub fn set_ssh_links(_on: bool) -> crate::error::AppResult<()> {
    Err(crate::error::AppError::validation(
        "This is a Windows setting.",
    ))
}

#[cfg(not(windows))]
pub fn set_autostart(_on: bool) -> crate::error::AppResult<()> {
    Err(crate::error::AppError::validation(
        "This is a Windows setting.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_match_what_the_installer_writes() {
        let exe = r"C:\Users\me\AppData\Local\Dalan\dalan-app.exe";
        assert_eq!(
            ssh_command(exe),
            r#""C:\Users\me\AppData\Local\Dalan\dalan-app.exe" --open-url "%1""#
        );
        assert_eq!(
            autostart_command(exe),
            r#""C:\Users\me\AppData\Local\Dalan\dalan-app.exe" --minimized"#
        );
    }

    #[test]
    fn a_handler_is_recognised_by_its_program() {
        assert_eq!(
            exe_name_of(r#""C:\Program Files\PuTTY\putty.exe" "%1""#),
            "putty.exe"
        );
        assert_eq!(exe_name_of(r"C:\Tools\other.exe %1"), "other.exe");
        assert_eq!(exe_name_of(""), "");
        assert!(is_our_command(
            r#""D:\Apps\Dalan\DALAN-APP.EXE" --open-url "%1""#,
            "dalan-app.exe"
        ));
        assert!(!is_our_command(
            r#""C:\Tools\putty.exe" "%1""#,
            "dalan-app.exe"
        ));
    }

    #[test]
    #[cfg(not(windows))]
    fn off_windows_there_is_nothing_to_change() {
        let info = get().unwrap();
        assert!(!info.supported);
        assert!(set_ssh_links(true).is_err());
        assert!(set_autostart(true).is_err());
    }
}

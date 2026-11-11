; Dalan's English text for the installer and uninstaller. Same string ids as Tauri's English.nsh
; (see English.upstream.nsh and UPSTREAM.md), reworded in the voice of replica/brand.md, plus our own.
; Copy source: replica/windows-installer/spec.md section 12. English only.

; --- Welcome and finish ---
LangString welcomeTitle ${LANG_ENGLISH} "Welcome to Dalan"
LangString welcomeText ${LANG_ENGLISH} "This sets up Dalan ${VERSION}, a calm desktop client for your servers: SSH, files and tunnels in one place.$\r$\n$\r$\nIt installs Dalan for you only, without administrator rights. For everyone on this PC, use the all-users installer.$\r$\n$\r$\nYour vault, keys and passwords stay on this computer. Setup doesn't touch them."
LangString welcomeTextAllUsers ${LANG_ENGLISH} "This sets up Dalan ${VERSION} for everyone on this PC. Each person keeps their own vault and settings.$\r$\n$\r$\nYour vault, keys and passwords stay on this computer. Setup doesn't touch them."
LangString finishTitle ${LANG_ENGLISH} "Dalan is ready"
LangString finishText ${LANG_ENGLISH} "Dalan ${VERSION} is installed."
LangString finishRun ${LANG_ENGLISH} "Open Dalan now"
LangString finishHint ${LANG_ENGLISH} "The first time, Dalan asks you to choose a master password. Keep it in your password manager: there's no way to recover it."
; Until the extras page arrives (phase W2), the desktop shortcut is the finish page's second checkbox.
LangString createDesktop ${LANG_ENGLISH} "Add a desktop shortcut"

; --- Already installed (the reinstall page) ---
LangString alreadyInstalled ${LANG_ENGLISH} "Dalan is already installed"
LangString installedVersionSub ${LANG_ENGLISH} "Version $ExistingVersion is on this PC."
LangString alreadyInstalledLong ${LANG_ENGLISH} "Dalan ${VERSION} is already installed. Choose what to do next, then click Next."
LangString addOrReinstall ${LANG_ENGLISH} "Reinstall Dalan"
LangString addOrReinstallDesc ${LANG_ENGLISH} "Replaces the program files. Keeps your vault, settings and the extras you chose."
LangString uninstallApp ${LANG_ENGLISH} "Remove Dalan"
LangString uninstallAppDesc ${LANG_ENGLISH} "Runs the uninstaller. You choose what happens to your vault and settings."
LangString reinstallUpdate ${LANG_ENGLISH} "Update to ${VERSION} (recommended)"
LangString reinstallUpdateDesc ${LANG_ENGLISH} "Keeps your vault, settings and the extras you chose."
LangString reinstallRemove ${LANG_ENGLISH} "Remove Dalan first, then install"
LangString reinstallRemoveDesc ${LANG_ENGLISH} "Your vault and settings are kept unless you choose otherwise."
LangString newerVersionInstalled ${LANG_ENGLISH} "A newer Dalan ($ExistingVersion) is already installed. Setup won't replace it."
LangString dontUninstall ${LANG_ENGLISH} "Keep the installed version"
LangString dontUninstallDowngrade ${LANG_ENGLISH} "Keep the newer version (Setup doesn't install older versions over it)"
; Tauri's own text for these is kept under the same ids so a template upgrade can't leave a string unset.
LangString chooseMaintenanceOption ${LANG_ENGLISH} "Choose what to do next."
LangString chooseHowToInstall ${LANG_ENGLISH} "Choose how you want to install Dalan."
LangString olderOrUnknownVersionInstalled ${LANG_ENGLISH} "An $R4 version of Dalan is installed on this PC."
LangString uninstallBeforeInstalling ${LANG_ENGLISH} "Remove Dalan first, then install"
LangString older ${LANG_ENGLISH} "older"
LangString unknown ${LANG_ENGLISH} "unknown"
LangString silentDowngrades ${LANG_ENGLISH} "A newer Dalan is already installed, and Setup doesn't replace it silently. Run Setup without /S to choose what to do.$\n"
LangString unableToUninstall ${LANG_ENGLISH} "Dalan couldn't be removed."

; --- Dalan is open ---
LangString appRunning ${LANG_ENGLISH} "Dalan is open. Close it to continue, then run Setup again."
LangString appRunningOkKill ${LANG_ENGLISH} "Dalan is open.$\nClick OK to close it now. Anything running in its terminals stops."
LangString failedToKillApp ${LANG_ENGLISH} "Dalan couldn't be closed. Close it yourself, then run Setup again."

; --- WebView2 (the part of Windows that draws Dalan's window) ---
LangString installingWebview2 ${LANG_ENGLISH} "Installing Microsoft Edge WebView2..."
LangString webview2AbortError ${LANG_ENGLISH} "WebView2 couldn't be installed, and Dalan can't run without it. Restart Setup to try again."
LangString webview2DownloadError ${LANG_ENGLISH} "WebView2 couldn't be downloaded: $0"
LangString webview2DownloadSuccess ${LANG_ENGLISH} "WebView2 downloaded."
LangString webview2Downloading ${LANG_ENGLISH} "Downloading WebView2..."
LangString webview2InstallError ${LANG_ENGLISH} "WebView2 setup failed with exit code $1."
LangString webview2InstallSuccess ${LANG_ENGLISH} "WebView2 installed."

; --- Uninstaller (the data page and the done page arrive with phase W2) ---
LangString deleteAppData ${LANG_ENGLISH} "Also delete my vault and settings"

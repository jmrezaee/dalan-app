; Dalan: hooks that Tauri's template calls (bundle > windows > nsis > installerHooks).
; This file is included near the top of installer.nsi, before the template's defines, so it only holds macros. The
; work is in options-page.nsh and data-page.nsh, which installer.nsi includes later; DALAN_DIR tells it where they are
; (this file's own folder, because the bundler copies the template elsewhere).
!define DALAN_DIR "${__FILEDIR__}"

; After the files are installed: shortcuts, ssh:// links, autostart, remembered choices.
!macro NSIS_HOOK_POSTINSTALL
  Call DalanApplyOptions
!macroend

; After the files are removed: the extras' registry keys.
!macro NSIS_HOOK_POSTUNINSTALL
  Call un.DalanPostUninstall
!macroend

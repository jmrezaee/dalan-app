; Dalan: the installer side of the extras (replica/windows-installer/spec.md sections 4, 5 and 7.3).
;   - the options: defaults, then the choices saved by the last install, then the command line (/STARTMENU=0|1,
;     /DESKTOP=0|1, /SSHLINKS=0|1, /AUTOSTART=0|1)
;   - the "Choose extras" page (nsDialogs), with detection of another app that opens ssh:// links
;   - what the installer writes: shortcuts (the template's functions do the work), the ssh:// handler, autostart,
;     and the remembered choices
;   - the guard against installing both editions on one PC
; Included by installer.nsi after its defines (hooks.nsh sets DALAN_DIR). Everything here runs in the installer;
; the uninstaller's half is data-page.nsh.

!include LogicLib.nsh
!include nsDialogs.nsh

!define DALAN_INSTALLER_KEY "Software\Dalan\Installer"
!define DALAN_CAPS_KEY "Software\Dalan\Capabilities"
!define DALAN_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

; The four choices (1 = yes, 0 = no).
Var OptStartMenu
Var OptDesktop
Var OptSsh
Var OptAutostart
; Page controls, and the name of the app that already opens ssh:// links (for the help text).
Var DalanChkStart
Var DalanChkDesktop
Var DalanChkSsh
Var DalanChkAutostart
Var DalanSshName
Var DalanSshNote
Var DalanUserName

; ssh:// links: what the installer writes (HKCU in both editions, because it is a per-user choice) and what it
; removes. The handler is only removed when its command is exactly the one we wrote, so another app's handler stays.
; Defined twice, for the installer and (with the "un." prefix) for the uninstaller.
!macro DalanDefineSshFunctions PREFIX
  Function ${PREFIX}DalanSshCommand
    StrCpy $R8 "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --open-url $\"%1$\""
  FunctionEnd

  Function ${PREFIX}DalanRemoveSsh
    Call ${PREFIX}DalanSshCommand
    ReadRegStr $R7 HKCU "Software\Classes\ssh\shell\open\command" ""
    ${If} $R7 == $R8
      DeleteRegKey HKCU "Software\Classes\ssh"
    ${EndIf}
    ReadRegStr $R7 HKCU "Software\Classes\Dalan.ssh\shell\open\command" ""
    ${If} $R7 == $R8
      DeleteRegKey HKCU "Software\Classes\Dalan.ssh"
      DeleteRegKey HKCU "${DALAN_CAPS_KEY}"
      DeleteRegValue HKCU "Software\RegisteredApplications" "${PRODUCTNAME}"
    ${EndIf}
  FunctionEnd
!macroend
!insertmacro DalanDefineSshFunctions ""
!insertmacro DalanDefineSshFunctions "un."

; Reads one choice: the value saved by the last install (registry), then the command line option.
!macro DalanLoadChoice REGNAME CMDNAME VAR
  ClearErrors
  ReadRegDWORD $0 SHCTX "${DALAN_INSTALLER_KEY}" "${REGNAME}"
  ${IfNot} ${Errors}
    ${If} $0 = 0
      StrCpy ${VAR} 0
    ${Else}
      StrCpy ${VAR} 1
    ${EndIf}
  ${EndIf}
  ClearErrors
  ${GetOptions} $CMDLINE "/${CMDNAME}=" $0
  ${IfNot} ${Errors}
    ${If} $0 == "0"
      StrCpy ${VAR} 0
    ${ElseIf} $0 == "1"
      StrCpy ${VAR} 1
    ${EndIf}
  ${EndIf}
!macroend

; Called from .onInit once SHCTX is known (and /NS has been read).
Function DalanInitOptions
  StrCpy $OptStartMenu 1
  StrCpy $OptDesktop 0
  StrCpy $OptSsh 0
  StrCpy $OptAutostart 0
  !insertmacro DalanLoadChoice "StartMenu" "STARTMENU" $OptStartMenu
  !insertmacro DalanLoadChoice "Desktop" "DESKTOP" $OptDesktop
  !insertmacro DalanLoadChoice "SshLinks" "SSHLINKS" $OptSsh
  !insertmacro DalanLoadChoice "Autostart" "AUTOSTART" $OptAutostart
  ; /NS (Tauri's "no shortcuts") wins over everything for the two shortcuts
  ${If} $NoShortcutMode = 1
    StrCpy $OptStartMenu 0
    StrCpy $OptDesktop 0
  ${EndIf}
FunctionEnd

; Called from .onInit: one edition per PC. The other edition's uninstall key sits in the other hive.
Function DalanGuardEdition
  StrCpy $R9 ""
  !if "${INSTALLMODE}" == "currentUser"
    ReadRegStr $0 HKLM "${UNINSTKEY}" "UninstallString"
    ${If} $0 != ""
      StrCpy $R9 "$(otherEditionUser)"
    ${EndIf}
  !else if "${INSTALLMODE}" == "perMachine"
    ReadRegStr $0 HKCU "${UNINSTKEY}" "UninstallString"
    ${If} $0 != ""
      System::Call 'advapi32::GetUserName(t .r1, *i 256 r2) i .r3'
      StrCpy $DalanUserName $1
      StrCpy $R9 "$(otherEditionAll)"
    ${EndIf}
  !endif
  ${If} $R9 != ""
    ${If} ${Silent}
      System::Call 'kernel32::AttachConsole(i -1)i.r0'
      ${If} $0 <> 0
        System::Call 'kernel32::GetStdHandle(i -11)i.r0'
        System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; red
        FileWrite $0 "$R9$\n"
      ${EndIf}
    ${Else}
      MessageBox MB_ICONSTOP "$R9"
    ${EndIf}
    SetErrorLevel 5
    Abort
  ${EndIf}
FunctionEnd

; $0 = a registry command line such as `"C:\Program Files\PuTTY\putty.exe" "%1"`; returns the file name in $0.
Function DalanExeNameOf
  StrCpy $1 $0 1
  ${If} $1 == '"'
    StrCpy $0 $0 "" 1
    ${StrLoc} $2 $0 '"' ">"
  ${Else}
    ${StrLoc} $2 $0 " " ">"
  ${EndIf}
  ${If} $2 != ""
    StrCpy $0 $0 $2
  ${EndIf}
  ; keep what follows the last backslash
  StrLen $3 $0
  dalan_name_loop:
    ${If} $3 <= 0
      Return
    ${EndIf}
    IntOp $3 $3 - 1
    StrCpy $4 $0 1 $3
    ${If} $4 == "\"
      IntOp $3 $3 + 1
      StrCpy $0 $0 "" $3
      Return
    ${EndIf}
    Goto dalan_name_loop
FunctionEnd

; Sets $DalanSshNote to the text for the ssh:// checkbox's second label, or "" when nothing else claims the links.
Function DalanDetectSshHandler
  StrCpy $DalanSshNote ""
  StrCpy $DalanSshName ""
  ; 1. The person picked an app in Windows Settings: that choice is hash-protected, we can only say so.
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\Shell\Associations\UrlAssociations\ssh\UserChoice" "ProgId"
  ${If} $0 != ""
  ${AndIf} $0 != "Dalan.ssh"
    ReadRegStr $1 HKCR "$0" ""
    ${If} $1 == ""
      StrCpy $1 $0
    ${EndIf}
    StrCpy $DalanSshName $1
    StrCpy $DalanSshNote "$(optSshPinned)"
    Return
  ${EndIf}
  ; 2. Another app registered the scheme: the per-user key first, then the machine-wide one.
  ReadRegStr $0 HKCU "Software\Classes\ssh\shell\open\command" ""
  ${If} $0 == ""
    ReadRegStr $0 HKCR "ssh\shell\open\command" ""
  ${EndIf}
  ${If} $0 != ""
    ${StrLoc} $2 $0 "${MAINBINARYNAME}.exe" ">"
    ${If} $2 == ""
      Call DalanExeNameOf
      StrCpy $DalanSshName $0
      StrCpy $DalanSshNote "$(optSshOther)"
    ${EndIf}
  ${EndIf}
FunctionEnd

; The "Choose extras" page. It is skipped when the options come from elsewhere: a silent or passive install, or an
; update (the saved choices stay as they are).
Function DalanOptionsPage
  ${If} ${Silent}
  ${OrIf} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    Abort
  ${EndIf}

  Call DalanDetectSshHandler
  !insertmacro MUI_HEADER_TEXT "$(optionsTitle)" "$(optionsSub)"
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${IfThen} $(^RTL) = 1 ${|} nsDialogs::SetRTL $(^RTL) ${|}

  ${NSD_CreateCheckbox} 0 2u 100% 10u "$(optStartMenu)"
  Pop $DalanChkStart
  ${NSD_CreateCheckbox} 0 16u 100% 10u "$(optDesktop)"
  Pop $DalanChkDesktop
  ${NSD_CreateCheckbox} 0 30u 100% 10u "$(optSsh)"
  Pop $DalanChkSsh
  ${NSD_CreateLabel} 12u 42u -12u 18u "$(optSshHelp)"
  Pop $0
  ${If} $DalanSshNote != ""
    ${NSD_CreateLabel} 12u 62u -12u 18u "$DalanSshNote"
    Pop $0
  ${EndIf}
  ${NSD_CreateCheckbox} 0 86u 100% 10u "$(optAutostart)"
  Pop $DalanChkAutostart
  ${NSD_CreateLabel} 12u 98u -12u 10u "$(optAutostartHelp)"
  Pop $0

  ${If} $OptStartMenu = 1
    ${NSD_Check} $DalanChkStart
  ${EndIf}
  ${If} $OptDesktop = 1
    ${NSD_Check} $DalanChkDesktop
  ${EndIf}
  ${If} $OptSsh = 1
    ${NSD_Check} $DalanChkSsh
  ${EndIf}
  ${If} $OptAutostart = 1
    ${NSD_Check} $DalanChkAutostart
  ${EndIf}
  ${NSD_SetFocus} $DalanChkStart
  nsDialogs::Show
FunctionEnd

Function DalanOptionsLeave
  ${NSD_GetState} $DalanChkStart $OptStartMenu
  ${NSD_GetState} $DalanChkDesktop $OptDesktop
  ${NSD_GetState} $DalanChkSsh $OptSsh
  ${NSD_GetState} $DalanChkAutostart $OptAutostart
FunctionEnd

; Removes a shortcut of ours that was ticked off in a reinstall (never a shortcut that points elsewhere).
!macro DalanRemoveShortcutIfOurs PATH
  !insertmacro IsShortcutTarget "${PATH}" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 = 1
    !insertmacro UnpinShortcut "${PATH}"
    Delete "${PATH}"
  ${EndIf}
!macroend

; Run after the files are installed (NSIS_HOOK_POSTINSTALL). The shortcuts themselves are made by the template's
; CreateOrUpdate...Shortcut functions, which look at $OptStartMenu and $OptDesktop.
Function DalanApplyOptions
  ${If} $UpdateMode = 1
    ; The updater keeps the saved choices untouched; only the version moves on.
    WriteRegStr SHCTX "${DALAN_INSTALLER_KEY}" "Version" "${VERSION}"
    Return
  ${EndIf}

  ; Shortcuts
  ${If} $OptStartMenu = 1
    DetailPrint "$(logStartMenu)"
  ${Else}
    !insertmacro DalanRemoveShortcutIfOurs "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  ${EndIf}
  ${If} $OptDesktop = 1
    DetailPrint "$(logDesktop)"
  ${Else}
    !insertmacro DalanRemoveShortcutIfOurs "$DESKTOP\${PRODUCTNAME}.lnk"
  ${EndIf}

  ; ssh:// links (spec 5.1). Never through Tauri's deep-link bundling: that registers unconditionally.
  ${If} $OptSsh = 1
    Call DalanSshCommand ; leaves the command in $R8
    WriteRegStr HKCU "Software\Classes\ssh" "" "URL:SSH link"
    WriteRegStr HKCU "Software\Classes\ssh" "URL Protocol" ""
    WriteRegStr HKCU "Software\Classes\ssh\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
    WriteRegStr HKCU "Software\Classes\ssh\shell\open\command" "" "$R8"
    ; The same, as a capable app, so Dalan shows in Settings > Default apps
    WriteRegStr HKCU "Software\RegisteredApplications" "${PRODUCTNAME}" "${DALAN_CAPS_KEY}"
    WriteRegStr HKCU "${DALAN_CAPS_KEY}" "ApplicationName" "${PRODUCTNAME}"
    WriteRegStr HKCU "${DALAN_CAPS_KEY}" "ApplicationDescription" "SSH client"
    WriteRegStr HKCU "${DALAN_CAPS_KEY}\URLAssociations" "ssh" "Dalan.ssh"
    WriteRegStr HKCU "Software\Classes\Dalan.ssh\shell\open\command" "" "$R8"
    DetailPrint "$(logSsh)"
  ${Else}
    Call DalanRemoveSsh
  ${EndIf}

  ; Start with Windows (spec 5.2): HKCU in both editions
  ${If} $OptAutostart = 1
    WriteRegStr HKCU "${DALAN_RUN_KEY}" "${PRODUCTNAME}" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --minimized"
    DetailPrint "$(logAutostart)"
  ${Else}
    DeleteRegValue HKCU "${DALAN_RUN_KEY}" "${PRODUCTNAME}"
  ${EndIf}

  ; Remembered choices (spec 5.3): the next install pre-fills the page with them
  WriteRegDWORD SHCTX "${DALAN_INSTALLER_KEY}" "StartMenu" $OptStartMenu
  WriteRegDWORD SHCTX "${DALAN_INSTALLER_KEY}" "Desktop" $OptDesktop
  WriteRegDWORD SHCTX "${DALAN_INSTALLER_KEY}" "SshLinks" $OptSsh
  WriteRegDWORD SHCTX "${DALAN_INSTALLER_KEY}" "Autostart" $OptAutostart
  !if "${INSTALLMODE}" == "perMachine"
    WriteRegStr SHCTX "${DALAN_INSTALLER_KEY}" "Edition" "allusers"
  !else
    WriteRegStr SHCTX "${DALAN_INSTALLER_KEY}" "Edition" "user"
  !endif
  WriteRegStr SHCTX "${DALAN_INSTALLER_KEY}" "Version" "${VERSION}"
FunctionEnd

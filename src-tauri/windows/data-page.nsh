; Dalan: the uninstaller's extras (replica/windows-installer/spec.md section 6).
;   - the "Your data" page: keep (the default) or delete the vault and settings. It replaces Tauri's "Delete app data"
;     checkbox and sets the same variable, $DeleteAppDataCheckboxState, which the template's uninstall section reads.
;   - the "done" page, which says where the data was kept (or that it was deleted)
;   - the clean-up of what options-page.nsh wrote (ssh:// links, remembered choices)
; Included by installer.nsi in place of the template's uninstall confirm page.

Var DalanRadioKeep
Var DalanRadioDelete

; Skipped when silent, passive or updating: nothing is deleted then (a silent uninstall keeps data; /DELETEDATA=1 deletes).
Function un.DalanDataPage
  ${If} ${Silent}
  ${OrIf} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    Abort
  ${EndIf}

  !insertmacro MUI_HEADER_TEXT "$(unTitle)" "$(unSub)"
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${IfThen} $(^RTL) = 1 ${|} nsDialogs::SetRTL $(^RTL) ${|}

  ${NSD_CreateLabel} 0 2u 100% 10u "$(unIntro)"
  Pop $0
  ${NSD_CreateRadioButton} 0 18u 100% 10u "$(unKeepTitle)"
  Pop $DalanRadioKeep
  ${NSD_CreateLabel} 12u 30u -12u 20u "$(unKeepDesc)"
  Pop $0
  ${NSD_CreateRadioButton} 0 56u 100% 10u "$(unDeleteTitle)"
  Pop $DalanRadioDelete
  ${NSD_CreateLabel} 12u 68u -12u 12u "$(unDeleteDesc)"
  Pop $0
  !if "${INSTALLMODE}" == "perMachine"
    ${NSD_CreateLabel} 12u 82u -12u 30u "$(unDeleteWarn)$\r$\n$(unDeleteOthers)"
  !else
    ${NSD_CreateLabel} 12u 82u -12u 30u "$(unDeleteWarn)"
  !endif
  Pop $0

  ; Keep is the default, and stays selected unless the person chose delete earlier on this page
  ${If} $DeleteAppDataCheckboxState = 1
    ${NSD_Check} $DalanRadioDelete
  ${Else}
    ${NSD_Check} $DalanRadioKeep
  ${EndIf}

  ; The page's button is "Uninstall"
  GetDlgItem $0 $HWNDPARENT 1
  SendMessage $0 ${WM_SETTEXT} 0 "STR:$(^UninstallBtn)"
  ${NSD_SetFocus} $DalanRadioKeep
  nsDialogs::Show
FunctionEnd

Function un.DalanDataLeave
  ${NSD_GetState} $DalanRadioDelete $DeleteAppDataCheckboxState
FunctionEnd

; The done page: the text depends on the choice, so it is set when the page shows.
Function un.DalanFinishPre
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    Abort
  ${EndIf}
FunctionEnd

Function un.DalanFinishShow
  ${If} $DeleteAppDataCheckboxState = 1
    StrCpy $R0 "$(unDoneDeleted)"
  ${Else}
    StrCpy $R0 "$(unDoneKept)"
  ${EndIf}
  SendMessage $mui.FinishPage.Text ${WM_SETTEXT} 0 "STR:$R0"
FunctionEnd

; Run at the end of the uninstall section (NSIS_HOOK_POSTUNINSTALL). Tauri's template has already removed the files,
; shortcuts, uninstall key and (when asked) the data folders. An update (/UPDATE) removes nothing here.
Function un.DalanPostUninstall
  ${If} $UpdateMode = 1
    Return
  ${EndIf}
  ; ssh:// links and the Default apps entry, only when they are still ours
  Call un.DalanRemoveSsh
  ; The remembered choices stay after a plain uninstall, so reinstalling picks up where it left off.
  ${If} $DeleteAppDataCheckboxState = 1
    DeleteRegKey SHCTX "${DALAN_INSTALLER_KEY}"
    DeleteRegKey /ifempty SHCTX "Software\Dalan"
  ${EndIf}
FunctionEnd

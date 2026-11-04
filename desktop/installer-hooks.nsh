; Thrall's own steps in the Windows installer (Tauri's NSIS hooks).
;
; Starting at sign-in is a value in the per-user Run key that points at this copy's desktop app.
; Uninstalling takes it away with the app, so nothing is left starting a program that isn't
; there. An upgrade uninstalls the old copy first, so the uninstaller also notes that it was on,
; and the installer turns it back on for the new copy and forgets the note.

!define THRALL_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define THRALL_RUN_VALUE "Thrall"
!define THRALL_NOTE_KEY "Software\Thrall"
!define THRALL_NOTE_VALUE "StartAtSignIn"

!macro NSIS_HOOK_PREUNINSTALL
  ReadRegStr $0 HKCU "${THRALL_RUN_KEY}" "${THRALL_RUN_VALUE}"
  StrCmp $0 '"$INSTDIR\thrall-desktop.exe" --hidden' 0 thrall_not_this_copy
    WriteRegDWORD HKCU "${THRALL_NOTE_KEY}" "${THRALL_NOTE_VALUE}" 1
    DeleteRegValue HKCU "${THRALL_RUN_KEY}" "${THRALL_RUN_VALUE}"
  thrall_not_this_copy:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ReadRegDWORD $0 HKCU "${THRALL_NOTE_KEY}" "${THRALL_NOTE_VALUE}"
  StrCmp $0 1 0 thrall_was_off
    WriteRegStr HKCU "${THRALL_RUN_KEY}" "${THRALL_RUN_VALUE}" '"$INSTDIR\thrall-desktop.exe" --hidden'
    DeleteRegValue HKCU "${THRALL_NOTE_KEY}" "${THRALL_NOTE_VALUE}"
    DeleteRegKey /ifempty HKCU "${THRALL_NOTE_KEY}"
  thrall_was_off:
!macroend

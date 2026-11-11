; Extra steps for the Tauri NSIS installer.

!macro NSIS_HOOK_PREUNINSTALL
  ; Tauri runs this hook before it closes the app, so close Pane (and its watchdog, the same exe) here,
  ; then bring the Windows taskbar back as it was.
  nsExec::Exec 'taskkill /F /IM "${MAINBINARYNAME}.exe"'
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --restore'
  ; "Start with Windows" entry, if the user turned it on.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
!macroend

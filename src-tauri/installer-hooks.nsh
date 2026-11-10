!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\lore-cli.exe" integrations disconnect claude'
    Pop $0
    nsExec::Exec '"$INSTDIR\lore-cli.exe" integrations disconnect codex'
    Pop $0
  ${EndIf}
!macroend

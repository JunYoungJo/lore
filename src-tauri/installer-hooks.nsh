!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\qmd-memory-cli.exe" disconnect claude'
    Pop $0
    nsExec::Exec '"$INSTDIR\qmd-memory-cli.exe" disconnect codex'
    Pop $0
  ${EndIf}
!macroend

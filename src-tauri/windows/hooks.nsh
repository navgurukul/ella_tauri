; Hooks into Tauri's NSIS installer (`bundle > windows > nsis > installerHooks`).
;
; Ella's two engines run as programs of their own from the install folder:
; engines\bin\llama\llama-server.exe and engines\bin\piper\piper.exe. Ella
; stops them when it quits, but the installer closes a running Ella by ending
; its process outright, so Ella never gets the chance, and Windows does not end
; a program's children with it. A llama-server left behind that way, or by a
; crash or Task Manager, keeps its exe and DLLs open, and the installer cannot
; write over them: "Error opening file for writing". By then Ella.exe itself
; has already been replaced, so a learner who aborts is left with a new Ella on
; an unfinished install.
;
; So before copying anything, close Ella the way the template is about to, then
; end whatever engine is still running.

!macro ELLA_STOP_ENGINES
  ; The template's own check, run here first so the engines are ended after
  ; Ella rather than from under it. It asks before closing Ella (an update
  ; closes it without asking) and does nothing if Ella is not running, so the
  ; template's check that follows finds nothing left to do.
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; By name, the way the template finds Ella itself, and only this user's: a
  ; per-user install belongs to the user running its installer.
  nsis_tauri_utils::KillProcessCurrentUser "llama-server.exe"
  Pop $R0
  nsis_tauri_utils::KillProcessCurrentUser "piper.exe"
  Pop $R1
  ; 2 means none was running. One that was holds its files open for a moment
  ; after it is told to end.
  ${If} $R0 <> 2
  ${OrIf} $R1 <> 2
    Sleep 500
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro ELLA_STOP_ENGINES
!macroend

; Uninstalling deletes the same files.
!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro ELLA_STOP_ENGINES
!macroend

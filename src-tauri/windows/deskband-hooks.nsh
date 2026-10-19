!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\deskband-install.ps1" -Action Install -InstallDirectory "$INSTDIR"' $0
  StrCmp $0 0 postinstall_done
  StrCmp $LANGUAGE 2052 0 +2
  Abort "CQBar 任务栏工具栏注册失败。"
  Abort "CQBar taskbar toolbar registration failed."
  postinstall_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait '"$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "$INSTDIR\deskband-install.ps1" -Action Uninstall -InstallDirectory "$INSTDIR"' $0
  StrCmp $0 0 preuninstall_done
  StrCmp $LANGUAGE 2052 0 +2
  Abort "CQBar 任务栏工具栏卸载失败。"
  Abort "CQBar taskbar toolbar removal failed."
  preuninstall_done:
!macroend

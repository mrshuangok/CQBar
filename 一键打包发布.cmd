@echo off
setlocal
cd /d "%~dp0"

powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\release.ps1"
set "RELEASE_EXIT_CODE=%ERRORLEVEL%"

echo.
if not "%RELEASE_EXIT_CODE%"=="0" (
  echo Release failed. Review the error above.
) else (
  echo Release package is ready.
)
echo.
pause
exit /b %RELEASE_EXIT_CODE%

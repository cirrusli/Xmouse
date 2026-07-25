@echo off
setlocal
set "XMOUSE_EXE=%~dp0latest\Xmouse.exe"
if not exist "%XMOUSE_EXE%" (
  echo Xmouse executable was not found in the latest folder.
  echo Restore latest\Xmouse.exe or extract the current portable package first.
  pause
  exit /b 1
)
start "" /D "%~dp0latest" "%XMOUSE_EXE%"
endlocal

; Glint installer (NSIS). Per-user, no admin rights.
; Build: makensis /DVERSION=0.1.0 installer\glint.nsi
; Output: dist\Glint-Setup-x64-<version>.exe

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef BIN
  !define BIN "..\target\release"
!endif

Name "Glint"
OutFile "..\dist\Glint-Setup-x64-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\Glint"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "Glint ${VERSION}"

!define MUI_ICON "..\assets\glint.ico"
!define MUI_UNICON "..\assets\glint.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\glint.exe"
!define MUI_FINISHPAGE_RUN_PARAMETERS "--background"
!define MUI_FINISHPAGE_RUN_TEXT "Start Glint now (Ctrl+Alt+S to snip)"
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Glint"
VIAddVersionKey "FileDescription" "Glint installer"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT License"

!define UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\Glint"
!define RUNKEY "Software\Microsoft\Windows\CurrentVersion\Run"

; Ask a running Glint to quit, then make sure it is gone, so its files can be replaced.
!macro StopGlint
  IfFileExists "$INSTDIR\glint.exe" 0 +3
    nsExec::Exec '"$INSTDIR\glint.exe" quit'
    Sleep 800
  nsExec::Exec 'taskkill /f /im glint-editor.exe'
  nsExec::Exec 'taskkill /f /im glint.exe'
!macroend

Section "Install"
  !insertmacro StopGlint
  SetOutPath "$INSTDIR"
  File "${BIN}\glint.exe"
  File "${BIN}\glint-editor.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Glint.lnk" "$INSTDIR\glint.exe" "" "$INSTDIR\glint.exe" 0
  ; A snipping tool should be ready after sign-in; turn it off from the tray menu.
  WriteRegStr HKCU "${RUNKEY}" "Glint" '"$INSTDIR\glint.exe" --background'
  WriteRegStr HKCU "${UNINST}" "DisplayName" "Glint"
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "Publisher" "Glint"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" "$INSTDIR\glint.exe"
  WriteRegStr HKCU "${UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINST}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINST}" "EstimatedSize" 12000
  ; Silent installs (updates) start Glint again right away.
  IfSilent 0 +2
    Exec '"$INSTDIR\glint.exe" --background'
SectionEnd

Section "Uninstall"
  !insertmacro StopGlint
  Delete "$INSTDIR\glint.exe"
  Delete "$INSTDIR\glint-editor.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Glint.lnk"
  DeleteRegValue HKCU "${RUNKEY}" "Glint"
  DeleteRegKey HKCU "${UNINST}"
  ; Settings in %APPDATA%\Glint and saved screenshots are kept.
SectionEnd

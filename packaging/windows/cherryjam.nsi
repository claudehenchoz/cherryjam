; CherryJam installer (NSIS 3).
;
; Build from the repository root:
;   makensis /DVERSION=1.2.3 /DOUTFILE=cherryjam-setup.exe packaging\windows\cherryjam.nsi
; Paths below are relative to this script's folder.

Unicode true
SetCompressor /SOLID lzma
ManifestDPIAware true

!include "MUI2.nsh"
!include "x64.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef OUTFILE
  !define OUTFILE "cherryjam-setup.exe"
!endif
!ifndef EXE
  !define EXE "..\..\target\release\cherryjam.exe"
!endif

!define APPNAME "CherryJam"
!define PUBLISHER "Claude Henchoz"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\CherryJam"

Name "${APPNAME}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APPNAME}"
InstallDirRegKey HKLM "Software\${APPNAME}" "InstallDir"
RequestExecutionLevel admin
BrandingText "${APPNAME} ${VERSION}"

; ---------------------------------------------------------------------------------------------
; Pages

!define MUI_ICON "..\..\icons\windows\cherryjam.ico"
!define MUI_UNICON "..\..\icons\windows\cherryjam.ico"
!define MUI_ABORTWARNING

!define MUI_WELCOMEPAGE_TITLE "Welcome to ${APPNAME} ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "Load a synth. Hit a key. Jam.$\r$\n$\r$\nThis will install ${APPNAME} on your computer. All you need is a VST3 instrument.$\r$\n$\r$\nClick Next to continue."
!define MUI_FINISHPAGE_RUN "$INSTDIR\cherryjam.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APPNAME} now"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ---------------------------------------------------------------------------------------------
; Install

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "${APPNAME} requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Section "${APPNAME}" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${EXE}"
  File "/oname=README.md" "..\..\README.md"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\${APPNAME}" "InstallDir" "$INSTDIR"

  ; Apps & features entry
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTKEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\cherryjam.exe"
  WriteRegStr HKLM "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTKEY}" "NoRepair" 1

  SetShellVarContext all
  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\cherryjam.exe"
SectionEnd

Section "Desktop shortcut" SecDesktop
  SetShellVarContext all
  CreateShortcut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\cherryjam.exe"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "The ${APPNAME} application and a Start menu shortcut."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Put a ${APPNAME} shortcut on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; ---------------------------------------------------------------------------------------------
; Uninstall (presets and settings in %APPDATA%\cherryjam are kept)

Function un.onInit
  SetRegView 64
FunctionEnd

Section "Uninstall"
  SetShellVarContext all
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"

  Delete "$INSTDIR\cherryjam.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "${UNINSTKEY}"
  DeleteRegKey HKLM "Software\${APPNAME}"
SectionEnd

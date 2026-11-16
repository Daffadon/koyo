Unicode true

!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.2.4"
!endif
!ifndef ICON
  !define ICON "../../koyo/assets/koyo.ico"
!endif
!ifndef EXE
  !define EXE "../../target/x86_64-pc-windows-msvc/release/koyo.exe"
!endif
!ifndef OUTFILE
  !define OUTFILE "../../koyo-windows-x64-setup.exe"
!endif

Name "Koyo"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Koyo"
InstallDirRegKey HKCU "Software\Koyo" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ABORTWARNING
!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Koyo" SecMain
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${EXE}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortCut "$SMPROGRAMS\Koyo.lnk" "$INSTDIR\koyo.exe" "" "$INSTDIR\koyo.exe" 0
  CreateShortCut "$DESKTOP\Koyo.lnk" "$INSTDIR\koyo.exe" "" "$INSTDIR\koyo.exe" 0

  WriteRegStr HKCU "Software\Koyo" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "DisplayName" "Koyo"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "Publisher" "Daffadon"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "DisplayIcon" "$INSTDIR\koyo.exe,0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$SMPROGRAMS\Koyo.lnk"
  Delete "$DESKTOP\Koyo.lnk"
  Delete "$INSTDIR\koyo.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Koyo"
  DeleteRegKey HKCU "Software\Koyo"
SectionEnd

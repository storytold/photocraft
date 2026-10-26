; Linux cross-build fallback: a per-user x64 development installer.
; Official, signed Windows releases continue to use package.ps1 and WiX/MSVC.
; makensis -DVERSION=0.5.0 -DDISPLAY_VERSION=0.5.0-dev.20261009 \
;   -DBIN_DIR=/absolute/stage -DOUT_FILE=/absolute/setup.exe \
;   -DICON_FILE=/absolute/photocraft.ico photocraft-dev.nsi
Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

!ifndef VERSION
  !error "VERSION is required"
!endif
!ifndef DISPLAY_VERSION
  !error "DISPLAY_VERSION is required"
!endif
!ifndef BIN_DIR
  !error "BIN_DIR is required"
!endif
!ifndef OUT_FILE
  !error "OUT_FILE is required"
!endif
!ifndef ICON_FILE
  !error "ICON_FILE is required"
!endif

Name "PhotoCraft ${DISPLAY_VERSION}"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\PhotoCraft-Dev"
InstallDirRegKey HKCU "Software\PhotoCraft\Development" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
Icon "${ICON_FILE}"
UninstallIcon "${ICON_FILE}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "PhotoCraft"
VIAddVersionKey /LANG=1033 "FileDescription" "PhotoCraft development installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${DISPLAY_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${DISPLAY_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright (c) the PhotoCraft authors"

!define MUI_ABORTWARNING
!define MUI_ICON "${ICON_FILE}"
!define MUI_UNICON "${ICON_FILE}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "German"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "PhotoCraft requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "PhotoCraft" Main
  SetOutPath "$INSTDIR"
  File "${BIN_DIR}/photocraft.exe"
  File "${BIN_DIR}/photocraft-cli.exe"
  File "${BIN_DIR}/LICENSES.txt"
  File "${BIN_DIR}/BUILD-INFO.json"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\PhotoCraft Development.lnk" "$INSTDIR\photocraft.exe"
  WriteRegStr HKCU "Software\PhotoCraft\Development" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "DisplayName" "PhotoCraft ${DISPLAY_VERSION} (Development)"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "DisplayVersion" "${DISPLAY_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "DisplayIcon" "$INSTDIR\photocraft.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "Publisher" "PhotoCraft contributors"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\PhotoCraft Development.lnk"
  Delete /REBOOTOK "$INSTDIR\photocraft.exe"
  Delete /REBOOTOK "$INSTDIR\photocraft-cli.exe"
  Delete "$INSTDIR\LICENSES.txt"
  Delete "$INSTDIR\BUILD-INFO.json"
  Delete /REBOOTOK "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoCraft-Development"
  DeleteRegKey HKCU "Software\PhotoCraft\Development"
SectionEnd

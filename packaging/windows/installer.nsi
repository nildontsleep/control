; Installateur Windows de control (NSIS) : par utilisateur, sans droits d'administrateur, comme la version Electron.
; Construire : makensis /DVERSION=1.0.0 /DEXE=chemin\control.exe /DOUT=chemin\control-1.0.0-setup.exe installer.nsi
;
; Mise à jour automatique : l'application lance « setup.exe /S --updated --force-run » ; l'installation est alors
; silencieuse et relance control une fois finie.

Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !error "VERSION manquant (/DVERSION=…)"
!endif
!ifndef EXE
  !error "EXE manquant (/DEXE=…)"
!endif
!ifndef OUT
  !define OUT "control-${VERSION}-setup.exe"
!endif

Name "control"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\control"
InstallDirRegKey HKCU "Software\control" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "control ${VERSION}"

!define MUI_ICON "..\..\build\icon.ico"
!define MUI_UNICON "..\..\build\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\control.exe"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "French"
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=${LANG_FRENCH} "ProductName" "control"
VIAddVersionKey /LANG=${LANG_FRENCH} "FileDescription" "Installateur de control"
VIAddVersionKey /LANG=${LANG_FRENCH} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_FRENCH} "LegalCopyright" "Copyright © 2026 nildontsleep"

!define UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\control"

Function .onInit
  ; Une mise à jour silencieuse attend que l'ancienne version se ferme (l'exécutable est verrouillé tant qu'il tourne).
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "--updated" $1
  IfErrors done
    Sleep 1500
  done:
FunctionEnd

Section "control" SecMain
  SetOutPath "$INSTDIR"
  ; Plusieurs essais : l'ancienne version peut mettre un instant à libérer son exécutable.
  StrCpy $2 0
  retry:
    ClearErrors
    File "/oname=control.exe" "${EXE}"
    IfErrors 0 copied
    IntOp $2 $2 + 1
    IntCmp $2 20 copied
    Sleep 500
    Goto retry
  copied:
  File "/oname=THIRD-PARTY.md" "..\..\THIRD-PARTY.md"
  WriteUninstaller "$INSTDIR\Uninstall control.exe"

  CreateShortCut "$SMPROGRAMS\control.lnk" "$INSTDIR\control.exe"
  CreateShortCut "$DESKTOP\control.lnk" "$INSTDIR\control.exe"

  WriteRegStr HKCU "Software\control" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST}" "DisplayName" "control"
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "Publisher" "nildontsleep"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" "$INSTDIR\control.exe"
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\Uninstall control.exe"'
  WriteRegStr HKCU "${UNINST}" "QuietUninstallString" '"$INSTDIR\Uninstall control.exe" /S'
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST}" "EstimatedSize" $0

  ; Mise à jour silencieuse : relancer control.
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "--force-run" $1
  IfErrors norun
    Exec '"$INSTDIR\control.exe"'
  norun:
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\control.exe"
  Delete "$INSTDIR\THIRD-PARTY.md"
  Delete "$INSTDIR\Uninstall control.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\control.lnk"
  Delete "$DESKTOP\control.lnk"
  DeleteRegKey HKCU "${UNINST}"
  DeleteRegKey HKCU "Software\control"
  ; Les données (bibliothèque, réglages) restent dans %APPDATA%\control.
SectionEnd

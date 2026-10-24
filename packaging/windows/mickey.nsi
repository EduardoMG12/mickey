; Mickey installer for Windows (NSIS). Built by .github/workflows/release.yml:
;   makensis /DVERSION=0.1.0 /DBIN=<folder with the two .exe> mickey.nsi
;
; Installs both programs, starts `mickey run` at login without a console
; window, and lets Mickey through the firewall on private networks only.

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef BIN
  !define BIN "."
!endif

Name "Mickey ${VERSION}"
OutFile "Mickey-Setup-${VERSION}.exe"
InstallDir "$PROGRAMFILES64\Mickey"
RequestExecutionLevel admin

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "PortugueseBR"

Section "Mickey"
  SetOutPath "$INSTDIR"
  ; A running Mickey keeps its files locked.
  nsExec::Exec 'taskkill /im mickey.exe /f'
  File "${BIN}\mickey.exe"
  File "${BIN}\mickey-ui.exe"
  File "start-hidden.vbs"
  ; Mickey's license, and the notices of the crates built into it.
  File "${BIN}\LICENSE.txt"
  File "${BIN}\THIRD-PARTY-LICENSES.txt"

  ; Start at login, hidden (mickey.exe is a console program), and with the
  ; highest privileges: Windows refuses injected input in windows running as
  ; administrator to a program that is not, and Mickey must reach every
  ; window. A scheduled task can start elevated without asking each time.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Mickey"
  ; Mickey makes the task itself, as the settings window does: elevated only
  ; when its folder is writable by administrators alone (the default, under
  ; Program Files), allowed on battery, through start-hidden.vbs.
  nsExec::Exec '"$INSTDIR\mickey.exe" autostart on'

  ; Peers connect to the target; private networks only, never public ones.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Mickey"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="Mickey" dir=in action=allow program="$INSTDIR\mickey.exe" profile=private'

  CreateDirectory "$SMPROGRAMS\Mickey"
  CreateShortcut "$SMPROGRAMS\Mickey\Mickey settings.lnk" "$INSTDIR\mickey-ui.exe" "settings"
  CreateShortcut "$SMPROGRAMS\Mickey\Uninstall Mickey.lnk" "$INSTDIR\uninstall.exe"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Mickey" "DisplayName" "Mickey"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Mickey" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Mickey" "UninstallString" '"$INSTDIR\uninstall.exe"'

  ; Start it now, elevated, as it will start at every login.
  nsExec::Exec 'schtasks /run /tn "Mickey"'
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /im mickey.exe /f'
  nsExec::Exec 'taskkill /im mickey-ui.exe /f'
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Mickey"
  nsExec::Exec '"$INSTDIR\mickey.exe" autostart off'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Mickey"'
  Delete "$INSTDIR\mickey.exe"
  Delete "$INSTDIR\mickey-ui.exe"
  Delete "$INSTDIR\start-hidden.vbs"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD-PARTY-LICENSES.txt"
  Delete "$INSTDIR\uninstall.exe"
  ; The trusted PCs of Mickey run as administrator (PRD 0002 Discoveries).
  RMDir /r "$INSTDIR\state"
  RMDir "$INSTDIR"
  RMDir /r "$SMPROGRAMS\Mickey"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Mickey"
  ; Keys, pairings and config.toml stay in %APPDATA%\mickey, like SSH keys.
SectionEnd

; Runs at the end of the NSIS uninstaller.
;
; Everything Steamware wrote for this user goes: settings (including the Hubcap key), caches, the download
; history, the installed plugin frontend and the WebView2 profile. The installer is per-user, so $APPDATA and
; $LOCALAPPDATA are the profile that ran the install.
;
; Deliberately NOT removed: files the user fetched into their own downloads folder (that is their content,
; not app state) and the loader DLLs in the Steam root. Restoring Steam's winmm.dll needs Steam closed and
; write access to Program Files, which an uninstaller cannot assume; removing the plugin from Steamware's
; Plugin page first is the supported path, and it stops Steam and restores the loader properly.
!macro NSIS_HOOK_POSTUNINSTALL
  SetShellVarContext current
  DetailPrint "Removing Steamware settings and data..."
  RMDir /r "$APPDATA\SteamwareDesktop"

  ; The plugin frontend lives here, along with data from the earlier Windows build.
  RMDir /r "$APPDATA\Steamware"

  ; WebView2 profile (cache, local storage) for this app id.
  RMDir /r "$LOCALAPPDATA\app.steamware.desktop"
!macroend

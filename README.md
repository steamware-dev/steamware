# Steamware v1.0.5

Windows 64-bit release of Steamware.

## Download

**Steamware_1.0.5_x64-setup.exe**

This release contains only the Windows installer. Source code is not distributed here, and no API keys or account credentials are included.

## Install

1. Download the setup file from this release.
2. Run it and follow the installer.
3. Launch Steamware from the Start menu or the desktop shortcut.

Windows SmartScreen may warn because this build is not code-signed. Choose **More info → Run anyway** only if the file came from this release and its SHA-256 matches the value below.

## What changed in 1.0.5

- Duplicate add requests for the same game are rejected while the first add is still checking or downloading.
- The download queue reuses an active manifest job for the same App ID instead of starting a second archive fetch.
- A failed or cancelled add can still be retried normally.
- DLC content is still acquired with the base game; the fix prevents the same game's archive from being generated repeatedly.
- The API also caches DepotBox manifest archives briefly so existing 1.0.4 installations do not need to reinstall for that server-side protection.

## Requirements

- Windows 10 or Windows 11, 64-bit
- Steam
- Microsoft Edge WebView2 Runtime (the installer bootstraps it if it is missing)

## Verification

SHA-256:

```text
97c12915c0724424bd67cc5431cc417293ef62d8a3431b36f875d341504a80d3
```

Only download this setup from the official release page. Do not use modified installers or share account credentials or API keys.

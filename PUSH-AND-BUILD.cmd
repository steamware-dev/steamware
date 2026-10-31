@echo off
REM Builds the .deb, .rpm, macOS Intel/Apple Silicon .dmg and Windows ARM64 installer.
REM
REM This pushes the source tree to GitHub, which starts the build automatically.
REM Sign-in: the first push opens a GitHub login window - approve it and the push finishes.
REM
REM The default target below is the existing PUBLIC release repo, which makes this source
REM public. Change REPO to a new PRIVATE repo name (create it on GitHub first, empty) if you
REM would rather not publish the code - the builds work exactly the same.
set REPO=steamware-dev/steamware

cd /d "%~dp0"
git remote set-url origin https://github.com/%REPO%.git
echo Pushing to %REPO% ...
git push -u origin main
echo.
if %ERRORLEVEL% NEQ 0 (
  echo Push did not complete. Sign in when the GitHub window appears, then run this again.
) else (
  echo Done. The build starts on its own:
  echo   https://github.com/%REPO%/actions
)
echo.
pause

<div align="center">

# Steamware

**A Windows desktop companion for your Steam library.**

Browse the store, inspect manifests and depots, keep builds, download fixes, and add a Steamware panel to Steam's store pages — without leaving your desktop.

[![Latest release](https://img.shields.io/github/v/release/steamware-dev/steamware?style=flat-square&label=release)](https://github.com/steamware-dev/steamware/releases/latest)
[![Discord](https://img.shields.io/discord/1280434554552717364?style=flat-square&logo=discord&logoColor=white&label=Discord)](https://discord.gg/sf55KprmUx)
[![Windows](https://img.shields.io/badge/Windows-10%2F11-0078D6?style=flat-square&logo=windows&logoColor=white)](#download)

[Download the latest release](https://github.com/steamware-dev/steamware/releases/latest) · [Join the Discord](https://discord.gg/sf55KprmUx) · [VirusTotal report](https://www.virustotal.com/gui/file/97c12915c0724424bd67cc5431cc417293ef62d8a3431b36f875d341504a80d3/detection)

</div>

---

## What is Steamware?

Steamware is a free desktop app for **Windows 10 and 11**. It puts the useful parts of Steam library management in one place: the store, your library, depots and DLC, manifest downloads, saved builds, game fixes, and an optional panel inside Steam itself.

It is built for people who like to see what Steam is doing under the hood — not for people who want another launcher.

| | |
| --- | --- |
| **Current release** | v1.0.5 for Windows 10/11 (64-bit) |
| **Catalog** | **158,000+ Steam games** (as of 6 November 2026) |
| **Community** | **11,000+ members** in the Steamware Discord |
| **Price** | Free |
| **Installer size** | 2.84 MB |
| **Steam account** | Steam must be installed and signed in; Steamware never asks for your Steam password |

> Steamware is not affiliated with Valve Corporation or Steam. Steam is a trademark of Valve.

---

## What you can do

- **Browse and search the Steam store** — featured games, new releases, top sellers, specials, genres, and game pages.
- **Search a 158,000+ game catalog** — find games by name or App ID and open the details that matter.
- **See the real library state** — installed games, partial downloads, install folders, sizes, and last-played times.
- **Inspect depots and DLC** — active depots, DLC entries, manifest IDs, build IDs, and what is switched on.
- **Add and manage builds** — save named build variants, switch between them, edit Lua, and keep a known-good setup.
- **Download manifests and fixes** — one queue for manifests and game fixes, with history and clear status for every job.
- **Keep the library tidy** — repair duplicate or stale library records so Steam shows what is actually installed.
- **Use the in-Steam plugin** — add a Steamware panel to Steam's store pages, with a button for the game you are looking at.
- **Switch loader modes** — choose the loader that matches your Steam setup, with an option to restore Steam's files.
- **Watch progress in one place** — queued, downloading, done, failed, and cancelled jobs stay visible.
- **Show what you are playing** — optional Discord Rich Presence for your current Steamware page.

## What Steamware does not do

- It does **not** host or distribute game files.
- It does **not** ask for, store, or send your Steam password.
- It does **not** create a Steam account or bypass Steam's sign-in.
- It does **not** install a background remote-access service.
- It does **not** hide what it writes: plugin and loader changes are listed in the app and can be removed.

---

## How it works

Steamware is a normal desktop application that works with the Steam installation already on your PC.

1. **It finds Steam.** Steamware reads Steam's install folder and library folders, or you can point it at a custom location in Settings.
2. **It reads your library.** Installed apps, manifests, install state, and depot details come from the files Steam already keeps on disk.
3. **You choose what to add.** When you request a game or a fix, Steamware downloads the manifest or fix you selected and writes the matching Lua or manifest files into Steam's local configuration.
4. **Steam picks up the changes.** Depending on the job, Steam may need one restart before the new entry appears.
5. **The plugin is optional.** The in-Steam panel is loaded into Steam's own store pages and talks to the Steamware desktop app over a local connection. If you do not install it, the desktop app works without it.
6. **The loader is optional.** Advanced loader modes are for custom Steam setups. Steamware shows what it writes, and the app can remove it again.

All game files remain Steam's responsibility. Steamware only handles the metadata, manifests, fixes, and local configuration you ask it to handle.

---

## Is Steamware a virus?

**No. Steamware is not malware.** It does not encrypt your files, steal passwords, install a hidden miner, or open a remote shell. It is an unsigned desktop tool that intentionally modifies local Steam configuration and, optionally, loads a panel into Steam.

That behaviour is exactly what makes some antivirus engines suspicious.

### VirusTotal — v1.0.5 installer

[**Open the full VirusTotal report for `Steamware_1.0.5_x64-setup.exe`**](https://www.virustotal.com/gui/file/97c12915c0724424bd67cc5431cc417293ef62d8a3431b36f875d341504a80d3/detection)

At the time of writing, VirusTotal showed:

| Result | Engines |
| --- | --- |
| **Flagged** | **5 of 71** |
| **Did not flag** | **66 of 71** |

The five flags were **Bkav Pro, DeepInstinct, Kaspersky, SecureAge, and Sophos**. Most are generic or machine-learning labels rather than a consistent malware family. The engines that did **not** flag the installer include Microsoft, Bitdefender, ESET, Google, Malwarebytes, Symantec, Trend Micro, Avast, and AVG.

**Why the flags?** The installer is unsigned, and Steamware's optional plugin and loader work by interacting with Steam's own files and interface. Heuristic scanners often treat that combination as suspicious. That explanation is not a promise that every engine will agree.

**Check it yourself:**

- File: `Steamware_1.0.5_x64-setup.exe`
- SHA-256: `97c12915c0724424bd67cc5431cc417293ef62d8a3431b36f875d341504a80d3`
- Verify in PowerShell:

```powershell
Get-FileHash .\Steamware_1.0.5_x64-setup.exe -Algorithm SHA256
```

A scan is a snapshot, not a guarantee. If the result makes you uncomfortable, do not run the file. If you want to inspect it first, use a virtual machine and download only from the official release page.

---

## Download

1. Open the [latest release](https://github.com/steamware-dev/steamware/releases/latest).
2. Download `Steamware_1.0.5_x64-setup.exe`.
3. Run the installer. Windows SmartScreen may warn because the build is unsigned; choose **More info → Run anyway** only if you downloaded it from this repository.
4. Confirm the SHA-256 above before running it.

**Requirements:** Windows 10 or 11, 64-bit, with Steam installed. The current public release is Windows only.

---

## Safety notes

- **Back up your Steam configuration first.** Steamware writes local Steam files when you ask it to add, fix, or switch something.
- **Use the official release page.** Do not run a copy from a random mirror or a re-uploaded installer.
- **Steam updates can break plugins and loaders.** If the in-Steam panel stops appearing after a Steam update, check Discord for the current compatibility note.
- **Use at your own risk.** Modifying Steam's local files is outside Valve's supported configuration.

---

## Community

The Steamware Discord is the main place for support, setup guides, release announcements, and help with a broken build.

**[Join the Steamware Discord →](https://discord.gg/sf55KprmUx)**

- **11,000+ members**
- Setup and troubleshooting help
- Release and compatibility announcements
- A place to share builds, fixes, and feedback

---

## Frequently asked questions

**Does Steamware need my Steam password?**  
No. Steam stays signed in on its own, and Steamware never asks for your password.

**Does Steamware download games?**  
It downloads manifests, fixes, and the metadata needed for the features you use. It does not host game files.

**Why does Windows SmartScreen warn me?**  
The installer is not code-signed. SmartScreen treats unknown publishers conservatively, especially for tools that modify other applications.

**Why does my antivirus flag it?**  
The app is unsigned and the optional plugin/loader interacts with Steam's process and local files. Check the VirusTotal report and hash above, and use a VM if you want to inspect it before running it.

**Does it work on Linux or macOS?**  
The current public release is Windows x64 only. Other platform builds are not part of this release.

**Can I remove the plugin or loader?**  
Yes. The plugin and loader can be removed from Steamware, and the app shows what it wrote.

**Will Steam updates break it?**  
Sometimes. Steam changes its internals regularly; check Discord before assuming a new issue is permanent.

**Is Steamware affiliated with Valve?**  
No. Steamware is an independent project. Steam and Valve are trademarks of Valve Corporation.

---

## Repository

This repository hosts the **public README and release downloads** for Steamware. It does not contain the Steamware application source code.

For support, use the [Discord](https://discord.gg/sf55KprmUx).

---

## Credits

Steamware is built and maintained by **jcimlas**.

Thanks to everyone in the [Discord](https://discord.gg/sf55KprmUx) who runs test builds and reports what breaks.

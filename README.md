# Timedivers Manager V2 (Rust)

Version manager for Steam games, rewritten in **Rust** with [gpui-kit](https://gpui-kit.com/).
A faithful port of the original Python/Tkinter [timedivers-manager-v2](https://github.com/leem919/timedivers-manager-v2):
download historical versions of a Steam game and switch between them instantly using NTFS junctions.

**This is a Windows-only tool.** It requires an NTFS drive for the game libraries (junctions) and a Chromium-based browser (Edge, Chrome, Brave, …) for scraping SteamDB.

## Setup
1. Create a new folder anywhere and place `timedivers-manager-rs.exe` into it. Build it yourself with `cargo build --release` or grab it from the releases.
2. Run the exe, add any steamapps folders that contain the Common folder, and then select it. (Don't select the Common folder itself.) You can add the steamapps folder from any drive you have games installed on.
3. Select the game, run the folder setup ("Setup Folders"), and update the manifests. You will need to sign into your Steam account on SteamDB in the browser window that opens, then click **Continue** in the scraper log.
4. Download a version. You can use the Steam Console or the Depot Downloader (instructions below).
5. If using the Steam Console, check if any depots are already downloaded; they might be for a different version and should be deleted to be safe.
6. Select "Open Steam Console" and wait for the Steam window to open and switch to the console tab.
7. Select "Copy" next to the first command, paste it into the console, and hit enter to start the download.
8. Wait for the console to say the depot download is complete, then repeat for the next depots.
9. Once the console says they're all downloaded, refresh and then select "Import Version".

The UI is available in English and 简体中文 — switch it in the top-right corner of the main window.

## Things to Know
1. Always check for game updates on Steam. If an update comes out, make sure that the Steam version is active in the version manager and then download the update. Updates cannot be easily skipped, and downloading an update while an old version is active will cause issues. Also set the Steam version as active if opting into or out of a beta.
2. Switch back to the Steam version when not actively playing for a while in case Steam does a file check or an update is released.
3. The scraping process for updating the manifests may appear stuck at some points. If it appears stuck for longer than a minute or two, close everything and try again.
4. This program does not prompt you for, or store, your password. That is all handled with the Depot Downloader and choosing to remember your password just passes the remember-password flag to it.

## Depot Downloader
The Depot Downloader is a little slower, but it's also a dedicated tool for downloading depots and will show you the download progress.
1. Download the latest windows-x64 version of the [Depot Downloader](https://github.com/SteamRE/DepotDownloader/releases) and place the exe in the same folder as the version manager.
2. Enter your Steam username and choose whether or not the downloader should remember your password.
3. Select a version and then download using the Depot Downloader.
4. You will be prompted for your Steam password. Account authentication is necessary for getting your licenses.

## Persistence
State is stored in [RON](https://github.com/ron-rs/ron) format under the standard
Windows data directories (short name `timedivers-manager-rs`; the reverse-DNS id `com.abyss-rei.timedivers-manager-rs` is used for the app identity / system notifications):

| Content | Location |
| --- | --- |
| `config.ron` — folders, username, per-game active version, UI language | `%APPDATA%\timedivers-manager-rs\` |
| `manifests/<appid>.ron` — scraped manifest ids | `%LOCALAPPDATA%\timedivers-manager-rs\manifests\` |
| `browser_data/` — Chrome profiles used for SteamDB scraping | `%LOCALAPPDATA%\timedivers-manager-rs\` |

`DepotDownloader.exe` is the one thing kept next to the exe, since it is an
external tool rather than persisted state.

## Building
1. Install the Rust toolchain (1.92+, MSVC) from [rustup.rs](https://rustup.rs).
2. Install [Visual Studio 2022 Build Tools](https://visualstudio.microsoft.com/downloads/) with the "Desktop development with C++" workload, and make sure `cmake` is on your PATH (both are required by gpui on Windows).
3. `cargo build --release` — the binary lands in `target/release/timedivers-manager-rs.exe`.

## Implementation notes
- **GUI**: [gpui-kit 0.7](https://gpui-kit.com/) (Zed's GPU-accelerated GPUI + gpui-component), built-in Dark theme, virtualized lists, modal dialogs and notifications.
- **i18n**: [rust-i18n](https://crates.io/crates/rust-i18n) with `locales/ui.yml` (en / zh-CN); component strings are localized too.
- **SteamDB scraping**: your own Chromium-based browser is launched with a remote-debugging port and a dedicated profile under `browser_data/`; [chromiumoxide](https://crates.io/crates/chromiumoxide) connects over CDP, and [scraper](https://crates.io/crates/scraper) parses the depot-manifests and patch-notes pages (5s pause between depots, closest-timestamp patch-title matching, missing-depot backfill — all mirroring the original).
- **Junctions**: the [junction](https://crates.io/crates/junction) crate; `Setup Folders` renames the install to `<dir>_steam` and creates a junction named `<dir>`, so switching a version only re-points the link. Switching to the "Steam Version" points it back at `_steam`.
- **Downloads**: historical depots come from the Steam console (`download_depot …`) with one-click copy + import, or from DepotDownloader via a generated batch file opened in its own console window.
- Tests cover the ACF parser, date normalization, manifest merging/backfill, path naming, browser-command building, depot-cache discovery and the file-import progress flow: `cargo test`.

Based on [Timedivers Manager](https://github.com/leem919/timedivers-manager) and its v2 rewrite.

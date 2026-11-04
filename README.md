<div align="center">

<img src="docs/banner.png" alt="Lightforge" width="640">

**A native macOS launcher for World of Warcraft on Apple Silicon.**

[![CI](https://github.com/matt-dance/lightforge-mac/actions/workflows/macos.yml/badge.svg)](https://github.com/matt-dance/lightforge-mac/actions/workflows/macos.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-E9AB28?style=flat-square)](LICENSE)
[![Status: beta](https://img.shields.io/badge/status-1.0.0--beta.9-00B4FF?style=flat-square)](RELEASES.md)
[![macOS](https://img.shields.io/badge/platform-macOS%20arm64-8A94A3?style=flat-square)](#install)

</div>

<img src="docs/play.jpg" alt="The Play page" width="100%">

---

## About this port

This repository ([matt-dance/lightforge-mac](https://github.com/matt-dance/lightforge-mac)) is an **unofficial community port** of [LightforgeProject/lightforge](https://github.com/LightforgeProject/lightforge) to macOS Apple Silicon. It is not an official LightforgeProject release and is **not affiliated with, endorsed by, or sponsored by Blizzard Entertainment**.

Licensed under [GPL-3.0](LICENSE), same as upstream. Installing World of Warcraft outside Battle.net may conflict with Blizzard's terms of service. **Use at your own risk.**

The original project is a Linux launcher. This fork downloads Blizzard's native macOS client and starts `World of Warcraft.app` directly — no Proton, Wine, or Battle.net desktop app.

**On Linux?** Use [the original Lightforge](https://github.com/LightforgeProject/lightforge).

## Requirements

| | |
| --- | --- |
| **Mac** | Apple Silicon (arm64) only. Intel Macs are not supported. |
| **OS** | macOS 13.0 Ventura or later (`LSMinimumSystemVersion` in the app bundle). |
| **Account** | A Battle.net account that can play the World of Warcraft title you install. Lightforge ships no game files. |

## Install

1. Download the signed and notarized `Lightforge-*-arm64.dmg` from
   **[GitHub Releases](https://github.com/matt-dance/lightforge-mac/releases/latest)**.
2. Open the disk image and drag **Lightforge** to **Applications**.
3. Eject the disk image, then open Lightforge from Applications.

Release builds are Developer ID signed and notarized, so Gatekeeper treats them like any other Mac app. You do **not** need `xattr -dr com.apple.quarantine`.

Ad-hoc CI artifacts (pull-request builds, or a release whose signing secrets were missing) are different: those are blocked until you right-click → **Open** the first time. Players should use the Releases DMG.

## First run

macOS skips the Linux setup wizard. Lightforge opens on the Play page.

1. **Confirm your region.** It is filled from the system locale on first launch (Americas and Oceania → `us`). Change it under **Settings → Downloads** before you install. Installed games keep the region written into their `.build.info`.
2. **Pick a World of Warcraft title** in the tile band and press **Install**. Lightforge downloads the native macOS (OSX) client into
   `~/Library/Application Support/Lightforge/games/World of Warcraft`.
3. **Press Play.** The game starts `World of Warcraft.app` itself. Sign in through the in-game Battle.net window (`Utils/BlizzardBrowser.app`). The Battle.net desktop app is not used on this port.

The first **World of Warcraft: Forever** launch can take a few minutes, with a beachball, while the client sets itself up. Lightforge shows a status notice for that first Play press: *“First launch can take a few minutes while WoW sets itself up; it is not frozen.”* Wait it out. Later launches skip the notice.

## Games

Lightforge installs, patches, and launches World of Warcraft titles itself. The first-run tiles are:

- World of Warcraft (Retail)
- World of Warcraft Classic (Classic Era)
- WoW Classic (progression)
- **World of Warcraft: Forever**
- WoW PTR

PTR and beta branches for those lines sit on the version picker next to Play. You can add more WoW products later from the games list.

**Not on this port:** other Battle.net titles (Diablo, Overwatch, Hearthstone, StarCraft, and the rest). Those are handed to the Battle.net client on Linux; there is no Wine prefix here, and **Launch via Battle.net** is not used.

## Keeping the game updated

When Blizzard publishes a new build, the Play button becomes **Update**. Press it, or use the cog menu → **Check for Updates**.

Every game update re-extracts `World of Warcraft.app` and `BlizzardBrowser.app`, then runs the same macOS fixup as a fresh install (CEF framework layout, executable bits, ad-hoc sign, clear quarantine). You should not need to repair by hand after a normal patch.

If Blizzard lists the next patch early, the Play page offers a **Pre-download**.

Lightforge itself is updated by downloading a new DMG from
[Releases](https://github.com/matt-dance/lightforge-mac/releases/latest)
and replacing the app in Applications.

## Where files live

```text
~/Library/Application Support/Lightforge/games/World of Warcraft
    the game (user-writable; no admin)

~/Library/Application Support/io.Tuxcore.Lightforge
    Lightforge's own data (config, artwork cache, logs)

~/Library/Application Support/io.Tuxcore.Lightforge/logs/lightforge-gui.log
    the launcher log
```

Logs rotate at 4 MiB and keep three old files. For more detail, start Lightforge with `LIGHTFORGE_LOG=debug`. The CLI binary at `Lightforge.app/Contents/MacOS/lightforge` writes `lightforge-cli.log` in the same folder.

## Troubleshooting

Start with **Settings → About → Diagnostics**, or run `lightforge doctor` from the CLI. Use **☰ → Report a bug** to send a report you can read before it leaves your machine.

**The tile still says Install, or login aborts after a failed extract.**  
Cog menu → **Locate / repair existing install**, or:

```bash
/Applications/Lightforge.app/Contents/MacOS/lightforge fixup --product wowf --region us
```

That repairs the `.app` / CEF tree in place (no 85 GB re-download). `--local` skips the CDN and only reassembles and signs what is already on disk. Use the product code and region that match your install (`wow`, `wow_classic_era`, `wowf`, …).

**Gatekeeper blocks the app.**  
You are not on a notarized Releases build. Prefer the [latest release](https://github.com/matt-dance/lightforge-mac/releases/latest). Ad-hoc copies need right-click → **Open** once.

**Play beachballs for a few minutes on the first Forever launch.**  
Expected. Wait for the in-game login window.

**Login never appears, or the client reaches login and then exits.**  
Run **Locate / repair existing install** so `BlizzardBrowser.app` and its CEF frameworks can launch.

**“Launch via Battle.net” errors.**  
On macOS, Play starts `World of Warcraft.app` directly. Use Play, not that button.

**Intel Mac, or another Blizzard game.**  
Out of scope for this port.

More Linux-oriented cases live in [TROUBLESHOOTING.md](TROUBLESHOOTING.md). Extra Mac packaging notes are in [packaging/macos/README.md](packaging/macos/README.md).

## Build from source

On a Mac with Xcode command-line tools and [Rust](https://rustup.rs/) 1.90 or newer:

```bash
git clone https://github.com/matt-dance/lightforge-mac.git
cd lightforge-mac
cargo build --release --locked --bins
packaging/macos/bundle.sh
packaging/macos/sign.sh sign target/dist/Lightforge.app
packaging/macos/dmg.sh
```

That writes `target/dist/Lightforge.app` and `target/dist/Lightforge-*-arm64.dmg`. `sign.sh` uses a Developer ID when the signing secrets are set; otherwise it ad-hoc signs. A local ad-hoc build needs right-click → **Open** the first time.

`cargo test --workspace --locked` is what CI runs. The GUI binary is `target/release/lightforge-gui`.

## Releases and signing

For maintainers. Players only need the Releases DMG.

A `v*` tag is the only path that publishes a GitHub release. [`.github/workflows/release.yml`](.github/workflows/release.yml) builds on `macos-14`, signs with a Developer ID and hardened runtime, notarizes (`notarytool submit --wait`), staples the `.app` and the DMG, then uploads `Lightforge-*-arm64.dmg` and `SHA256SUMS.txt`.

Pull requests never see the secrets. [`.github/workflows/macos.yml`](.github/workflows/macos.yml) smoke-tests an ad-hoc DMG on every PR. Signing and notarization on `main` / `workflow_dispatch` use the GitHub environment named `release`. Missing secrets fall back to ad-hoc signing; the job does not fail.

Certificate and App Store Connect API key setup is documented in
[packaging/macos/README.md](packaging/macos/README.md#signing-and-notarization).

## Documentation

| | |
| --- | --- |
| [INSTALL.md](INSTALL.md#macos-apple-silicon) | Extra Mac install notes |
| [packaging/macos/README.md](packaging/macos/README.md) | Bundle, DMG, Gatekeeper, signing |
| [TROUBLESHOOTING.md](TROUBLESHOOTING.md) | Known problems (mostly Linux; Mac cases are above) |
| [docs/CLI.md](docs/CLI.md) | Command-line tool |
| [TECHDOC.md](TECHDOC.md) | How the downloader works |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Bug reports and patches |
| [SECURITY.md](SECURITY.md) | Vulnerability reports |
| [DISCLAIMER.md](DISCLAIMER.md) | What leaves your machine |
| [RELEASES.md](RELEASES.md) | Upstream changelog |

## Legal

Lightforge is an independent, unofficial project, **not affiliated with, endorsed by, or sponsored by Blizzard Entertainment**. This Mac port is likewise unofficial and is not an official LightforgeProject release. World of Warcraft, Battle.net and Blizzard Entertainment are trademarks of Blizzard Entertainment, Inc.

Lightforge ships no game content: it downloads from Blizzard's own CDN with your own account, the same way their client does. Installing and playing outside Battle.net may conflict with Blizzard's terms of service. **Use at your own risk.** See [DISCLAIMER.md](DISCLAIMER.md).

Licensed under [GPL-3.0](LICENSE).

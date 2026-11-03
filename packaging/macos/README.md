# macOS package

Lightforge on Apple Silicon is a native `.app` that talks to Blizzard's CDN
the same way the Linux build does, then launches `World of Warcraft.app`
directly. Proton, Wine and a Battle.net prefix are not used.

## Build

On a Mac with Xcode command-line tools and a current Rust:

```bash
cargo build --release --locked --bins
packaging/macos/bundle.sh
codesign --force --deep --sign - target/dist/Lightforge.app
packaging/macos/dmg.sh
```

`bundle.sh` writes `Info.plist`, copies both binaries, and builds
`Lightforge.icns` from the PNG icons with `iconutil`. The GitHub Actions
workflow [`.github/workflows/macos.yml`](../../.github/workflows/macos.yml)
does this on `macos-14` and uploads the DMG.

## Install

1. Open the DMG.
2. Drag **Lightforge** onto **Applications**.
3. Eject the disk image.

The game files go in
`~/Library/Application Support/Lightforge/games/World of Warcraft` by
default (user-writable, no admin). That is the path a first install
already used; Lightforge keeps it. Lightforge's own data lives under
`~/Library/Application Support/io.Tuxcore.Lightforge`.

A complete install is recognised after relaunch (the `.app` is a
directory, not a missing binary). If the tile still says Install, the
cog menu's **Locate / repair existing install** adopts the folder on
disk. On an install that is already known it re-extracts the `.app` /
CEF files (not the 85 GB store), rebuilds `Versions/Current` inside
each `.framework`, sets `+x`, and ad-hoc signs the bundles so
BlizzardBrowser can show login. The download region is taken from the
system locale (Americas → `us`) and can be changed under Settings →
Downloads.

## Gatekeeper (ad-hoc signature)

The CI build is **ad-hoc signed**, not notarized, so macOS will warn that
the app is from an unidentified developer.

**First launch:** in Finder, right-click Lightforge → **Open** → **Open**.

Or strip the quarantine flag:

```bash
xattr -dr com.apple.quarantine /Applications/Lightforge.app
```

Then open it normally. This only needs doing once per copy of the app.

## Login

Play starts `World of Warcraft.app` with no extra arguments (the product
config lists none). The game shows Battle.net login itself, through
`Utils/BlizzardBrowser.app`. The Battle.net desktop app is **not**
required to sign in or play. Features that live only in that app (its
own shop, some friend-list chrome) are out of scope here; the in-game
shop is the game's.

If the client reaches the login screen and then aborts, run
**Locate / repair existing install** (or `lightforge fixup --product wowf --region us`).
That repairs the CEF framework layout in place.

## What this package does not do

- Intel Macs (x86_64). The DMG is arm64 only.
- Other Battle.net titles. Those still go through Battle.net on Linux;
  there is no Wine prefix on macOS.
- Notarization / Developer ID. That needs Apple credentials Lightforge
  does not have in CI.

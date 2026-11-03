# macOS package

Lightforge on Apple Silicon is a native `.app` that talks to Blizzard's CDN
the same way the Linux build does, then launches `World of Warcraft.app`
directly. Proton, Wine and a Battle.net prefix are not used.

This is an unofficial community port of
[LightforgeProject/lightforge](https://github.com/LightforgeProject/lightforge).
It is not an official LightforgeProject release and is not affiliated
with Blizzard Entertainment. Licensed under [GPL-3.0](../../LICENSE).
Installing World of Warcraft outside Battle.net may conflict with
Blizzard's terms of service. Use at your own risk.

## Build

On a Mac with Xcode command-line tools and a current Rust:

```bash
cargo build --release --locked --bins
packaging/macos/bundle.sh
packaging/macos/sign.sh sign target/dist/Lightforge.app
packaging/macos/dmg.sh
```

`bundle.sh` writes `Info.plist`, copies both binaries, and builds
`Lightforge.icns` from the PNG icons with `iconutil`.
`sign.sh` uses a Developer ID when the secrets below are set, otherwise
an ad-hoc signature.

The GitHub Actions workflow
[`.github/workflows/macos.yml`](../../.github/workflows/macos.yml)
builds and smoke-tests an ad-hoc DMG on every pull request. Signing and
notarization run only on `main`, `v*` tags, and `workflow_dispatch`,
behind the `release` environment, so pull requests (and forks) never
see the secrets. A `v*` tag is published by
[`.github/workflows/release.yml`](../../.github/workflows/release.yml).

## Install

1. Open the DMG.
2. Drag **Lightforge** onto **Applications**.
3. Eject the disk image.

**First run.** The download region is taken from the system locale
(Americas → `us`) and can be changed under Settings → Downloads. Press
**Play** to install or update Forever.

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
BlizzardBrowser can show login.

**Updates and patches.** Every game update re-extracts
`World of Warcraft.app` / `BlizzardBrowser.app` as a shallow TACT tree
and then runs the same fixup automatically. You should not need
**Locate / repair** after a normal patch.

## Gatekeeper

A **notarized Developer ID** build (a `v*` tag after the signing
secrets are set) opens like any other Mac app.

An **ad-hoc** build — pull-request artifacts, or a main/tag build
whose secrets were missing — is blocked by Gatekeeper.

**First launch of an unsigned build:** in Finder, right-click
Lightforge → **Open** → **Open**.

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

## Signing and notarization

Maintainers only. Players do not need any of this.

Signing runs on pushes to `main`, `v*` tags, and `workflow_dispatch`.
It never runs on `pull_request`. Secrets live on the GitHub
environment named `release`. Missing secrets fall back to ad-hoc
signing and print a notice; the job does not fail.

### Create the `release` environment and add the secrets

1. Open [matt-dance/lightforge-mac](https://github.com/matt-dance/lightforge-mac).
2. **Settings → Environments → New environment**.
3. Name it `release` (exact spelling).
4. Under **Environment secrets → Add secret**, add each of:

| Secret | Value |
| --- | --- |
| `MACOS_CERT_P12` | base64-encoded Developer ID Application `.p12` |
| `MACOS_CERT_PASSWORD` | password of that `.p12` |
| `MACOS_SIGN_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_API_KEY_P8` | the `.p8` file body (not base64) |
| `APPLE_API_KEY_ID` | the key id (10 characters) |
| `APPLE_API_ISSUER_ID` | the issuer UUID from App Store Connect |

Repo secrets under **Settings → Secrets and variables → Actions**
also work, but environment secrets are preferred so only jobs that
declare `environment: release` can read them.

Never paste a secret into a workflow log, a PR, or `sign.sh` argv.
The script writes the `.p8` to a temp file and deletes it; the
temporary keychain is deleted with `if: always()`.

### Developer ID Application certificate

1. In [Apple Developer](https://developer.apple.com/account/resources/certificates/list)
   → **Certificates, Identifiers & Profiles → Certificates → +**.
2. Choose **Developer ID Application**.
3. Create a CSR in Keychain Access (**Keychain Access → Certificate
   Assistant → Request a Certificate From a Certificate Authority**),
   upload it, download the certificate, and double-click to install it.
4. In Keychain Access, select the certificate + private key →
   **Export** as `.p12`. Set a password (`MACOS_CERT_PASSWORD`).
5. Encode it (the `-i` form does not wrap lines):

```bash
base64 -i DeveloperID.p12 | pbcopy
```

Paste that into `MACOS_CERT_P12`. `MACOS_SIGN_IDENTITY` is the
string Keychain Access shows for the certificate, for example
`Developer ID Application: Your Name (TEAMID)`.

### App Store Connect API key

1. In [App Store Connect](https://appstoreconnect.apple.com)
   → **Users and Access → Integrations → Team Keys → +**.
2. Name it something like `lightforge-notarize`. Role **Developer**
   or **App Manager** is enough for `notarytool`.
3. Download the `.p8` **once**. Copy the file body into
   `APPLE_API_KEY_P8` (including the `BEGIN` / `END` lines).
4. The key id is `APPLE_API_KEY_ID`. The issuer UUID at the top of
   the Integrations page is `APPLE_API_ISSUER_ID`.

### First signed release

After the six secrets are in the `release` environment, cut a tag
from `main` (do not do this until the secrets are in place):

```bash
git tag v1.0.0-beta.9
git push origin v1.0.0-beta.9
```

That runs `.github/workflows/release.yml`: build, sign with hardened
runtime and a secure timestamp, `notarytool submit --wait`, staple
the `.app` and the DMG, `spctl` / `stapler validate`, then publish a
real (non-draft) GitHub release with the DMG and `SHA256SUMS.txt`.

## What this package does not do

- Intel Macs (x86_64). The DMG is arm64 only.
- Other Battle.net titles. Those still go through Battle.net on Linux;
  there is no Wine prefix on macOS.

# Releases and updates

## CI and releases

| Workflow | Runs on | Does |
|---|---|---|
| `.github/workflows/ci.yml` | pull requests into `main` | rustfmt; clippy `-D warnings` for Linux and Windows, `cargo check` at the MSRV, actionlint; `cargo test` on Linux, macOS (with clippy) and Windows (gnullvm) |
| `.github/workflows/cd.yml` | pushes to `main`, `v*` tags, manual runs, pull requests into `main` touching the build | the guest payload (`build.sh --artifacts-only`, no Pioneer firmware), QEMU for every target, the universal `.dmg`, `.deb`/`.rpm`/AppImage for x86_64 and aarch64, the Windows x64 and arm64 installers; each smoke-tested, all kept on the run |

`main` takes changes through pull requests only: the four CI jobs must pass and
one review approve, which an admin can bypass. Only an admin creates `v*` tags.

A release is a `v<major>.<minor>.<patch>[-<pre>]` tag on `main`; the tag is
the version every artefact and the app itself carry (`CDJ3K_VERSION` at build
time), so nothing is bumped first. Its CD run notarizes the macOS build and puts
every artefact, with `SHA256SUMS.txt` and the update feeds
(`cdj3k-emu-update.json`, its `.sig`, `appcast.xml`), in a draft release,
published by hand; a version with a hyphen is a pre-release:

```bash
git tag v0.2.0 origin/main && git push origin v0.2.0
```

The macOS build is signed with the Developer ID held in the `release`
environment's secrets; pull requests do not get that environment and are signed
ad hoc, and a release refuses to run without it.

| Secret | Content |
|---|---|
| `MACOS_CERTIFICATE_P12_BASE64` | `base64 -i cert.p12`: the Developer ID Application certificate and key |
| `MACOS_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `MACOS_PROVISION_PROFILE_BASE64` | optional: `base64 -i cdj3k-emu.provisionprofile` (vmnet, virtual HID) |
| `NOTARY_API_KEY_BASE64` | `base64 -i AuthKey_<id>.p8`: an App Store Connect API key (Users and Access → Integrations, Developer role) |
| `NOTARY_API_KEY_ID` | that key's ID |
| `NOTARY_API_ISSUER_ID` | the issuer ID shown above the keys |
| `UPDATE_SIGNING_KEY` | the base64 32-byte Ed25519 seed that signs both update feeds (the format of Sparkle's `generate_keys -x`); its public half is `packaging/update-ed25519.pub`, which the app is built with and `Info.plist` carries as `SUPublicEDKey` |

## Updates

Every release carries two feeds, both read from `releases/latest/download/`,
which names the newest published release that is not a pre-release, so drafts
and pre-releases are never offered. Both carry the release's notes as they
stand when the release job runs (an edit to the draft reaches the feeds by
re-running that job), and both are signed with one Ed25519 key (`.github/scripts/sign.sh`), whose public half
is `packaging/update-ed25519.pub`.

- `appcast.xml` (`.github/scripts/appcast.sh`) is Sparkle's, for macOS. Sparkle
  2 (`Contents/Frameworks/Sparkle.framework`, loaded at runtime, release
  bundles only) draws its own window, checks the `.dmg`'s EdDSA signature
  against `SUPublicEDKey` and the new bundle's Developer ID against the running
  one, swaps the bundle and relaunches. Its `sparkle:version` is the release's
  CD run number, the bundle's `CFBundleVersion`, so `cd.yml`'s run counter
  must never start over, as it does for a renamed or recreated workflow. The app answers Sparkle's
  delegate: before the swap every other slot closes and the emulation stops,
  and Sparkle relaunches the app once.
- `cdj3k-emu-update.json` (`.github/scripts/update-index.sh`) and its
  signature `cdj3k-emu-update.json.sig`, the base64 Ed25519 signature over the
  file, are for Windows and Linux, which draw the update window themselves. The
  app checks the signature against the public key it was built with, and each
  package's size and SHA-256 against the index.

```json
{
  "schema": 1,
  "version": "0.4.0",
  "notes": "https://github.com/nsaintot/cdj3k-emu/releases/tag/v0.4.0",
  "release_notes": "## What's Changed\n* …",
  "packages": [
    { "os": "windows", "arch": "aarch64", "kind": "inno",
      "url": "https://github.com/…/v0.4.0/CDJ3K-Emulator-0.4.0-windows-arm64.exe",
      "size": 20491367, "sha256": "…" }
  ]
}
```

`os` and `arch` are Rust's `std::env::consts` names, with `universal` for a
bundle holding both slices. A package names its own `kind` in a
`package-kind` file beside the guest payload (`share/cdj3k-emu/` or
`Contents/Resources/`); a build without one, such as a developer's, is not
updated. `schema` goes up only for a change an older app would read wrongly.

The download and the update run while the emulation keeps going; the restart
is left with only the swap. Declining the restart puts the update in when the
last window quits.

| `kind` | Updated while running | At the restart | Prompt |
|---|---|---|---|
| `dmg` | Sparkle downloads and unpacks it | Sparkle's installer swaps the bundle | the admin password, only where the bundle's folder is not writable |
| `appimage` | the new image copied beside `$APPIMAGE` | a rename over it | none |
| `inno` | nothing: Windows cannot replace a loaded file | Setup with `/SILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE=1`, which shows only its progress window; it waits for `Global\cdj3k-emu`, then its launch entry starts the app once, unelevated (`/UPDATE=0` on a quit starts nothing) | UAC |
| `deb`, `rpm` | downloaded and checked, then moved to the XDG downloads folder; the package manager installs it | | none |

The automatic check runs when the first window opens (no other slot open) and
every 6 hours after, in the lowest-numbered open slot; a failed one retries
every 10 seconds for two minutes, then backs off to 30 minutes. On Windows and
Linux it stays quiet for a skipped version and anything older
(`update_skip_version`), and for 24 hours after Remind Me Later
(`update_remind_after`); with automatic download and install
(`update_auto_install`) it downloads and prepares the update without the
window, then offers the restart. All three are in the app-wide
`settings.txt`; Sparkle keeps its own.

Changing the signing key strands every build that carries the old public
key: they refuse the new feed and have to be updated by hand.

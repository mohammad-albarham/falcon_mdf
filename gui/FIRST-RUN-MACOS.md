# First run on macOS

falcon is **not signed or notarized**. macOS therefore refuses to run it the
first time, and the message it shows does not say "unsigned" — it says the app
is damaged, or that the developer cannot be verified. Nothing is damaged; the
binary simply carries no Apple Developer signature.

This file ships inside the macOS archive so that the message has an answer
beside it rather than on a web page someone has to go and find.

## Why it happens

Anything downloaded through a browser is tagged with a quarantine attribute.
Gatekeeper checks that tag against a signature. There is no signature here, so
it refuses. Nothing about the refusal is specific to this binary — every
unsigned download behaves the same way.

## Clearing it

Unzip the archive first, then, from the folder holding `falcon`:

```sh
xattr -d com.apple.quarantine falcon
```

`xattr: No such xattr` means the tag was never applied — the file did not come
through a browser — and there was nothing to clear. Either way, run it:

```sh
./falcon
```

Or, without the terminal: right-click `falcon` in Finder, choose **Open**, and
confirm once at the prompt. The choice is remembered, so this is a one-time
step for that copy of the binary. Double-clicking it *without* the right-click
gives the refusal again and no way past it.

## Checking what you downloaded

Every release archive is published with a `.sha256` file beside it. Since the
binary is unsigned, that checksum is the only integrity check available, and
it is worth using:

```sh
shasum -a 256 -c falcon-gui-macos-arm64.zip.sha256
```

`OK` means the download matches what the release workflow built.

## Which archive

- `falcon-gui-macos-arm64.zip` — Apple silicon (M1 and later)
- `falcon-gui-macos-x86_64.zip` — Intel Macs

`uname -m` reports `arm64` or `x86_64` if you are unsure.

## What you get

A single command-line-launchable binary, not a `.app`. It opens a normal
window; it just has no Finder bundle, so double-clicking it in Finder opens a
Terminal window alongside the viewer. To build a proper `Falcon.app` from
source instead, see `PACKAGING.md` in the repository — `cargo bundle` is
already configured for it.

Everything else about using the viewer is in `RUNNING.md`, shipped beside this
file.

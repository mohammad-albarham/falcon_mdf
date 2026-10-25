# First run on macOS

falcon is **not signed or notarized**. macOS therefore refuses to run it the
first time, and the message it shows does not say "unsigned" — it says the app
is damaged, or that the developer cannot be verified. Nothing is damaged; the
binary simply carries no Apple Developer signature.

This file ships inside the macOS archive and disk image so that the message has an answer
beside it rather than on a web page someone has to go and find.

## Why it happens

Anything downloaded through a browser is tagged with a quarantine attribute.
Gatekeeper checks that tag against a signature. There is no signature here, so
it refuses. Nothing about the refusal is specific to this binary — every
unsigned download behaves the same way.

## Clearing it: the disk image (`Falcon.app`)

Drag `Falcon.app` onto the `Applications` link in the window the `.dmg` opens,
then clear the tag from the copy you just made:

```sh
xattr -dr com.apple.quarantine /Applications/Falcon.app
```

Then open it from Launchpad, Spotlight or Finder like any other app.

Without the terminal: open it once and dismiss the refusal, then go to
**System Settings → Privacy & Security**, scroll to the message about Falcon
and choose **Open Anyway**. From macOS 15 on, this is the only route through
Finder: the old right-click → **Open** shortcut no longer gets past the
refusal.

The first time it opens a file in Desktop, Documents or Downloads, macOS asks
whether Falcon may read that folder. That prompt is the standard one every app
gets, not a sign that anything is wrong; allow it or the file will not open.

## Clearing it: the zip (`falcon` binary)

Unzip the archive first, then, from the folder holding `falcon`:

```sh
xattr -d com.apple.quarantine falcon
```

`xattr: No such xattr` means the tag was never applied — the file did not come
through a browser — and there was nothing to clear. Either way, run it:

```sh
./falcon
```

Or, without the terminal: double-click `falcon`, dismiss the refusal, and use
**Open Anyway** in System Settings → Privacy & Security as above. On macOS 14
and older, right-click → **Open** and confirming once does the same. The
choice is remembered, so this is a one-time step for that copy of the binary.

## Checking what you downloaded

Every release archive is published with a `.sha256` file beside it. Since the
binary is unsigned, that checksum is the only integrity check available, and
it is worth using:

```sh
shasum -a 256 -c falcon-gui-macos-arm64.zip.sha256
```

`OK` means the download matches what the release workflow built.

## Which download

- `falcon-gui-macos-arm64.dmg` — Apple silicon (M1 and later), as `Falcon.app`
- `falcon-gui-macos-arm64.zip` — Apple silicon, as a bare `falcon` binary
- `falcon-gui-macos-x86_64.zip` — Intel Macs, as a bare `falcon` binary

`uname -m` reports `arm64` or `x86_64` if you are unsure.

## What you get

The `.dmg` holds `Falcon.app`: it has an icon, lives in Applications, and opens
from Finder without a Terminal window. It does not yet open `.mf4` files
double-clicked in Finder; drop them on its window or use **Open File…**.

The zip holds a single command-line-launchable binary, not a `.app`. It opens
a normal window; it just has no Finder bundle, so double-clicking it in Finder
opens a Terminal window alongside the viewer. It is the one to use from a
script or the terminal (`falcon measurement.mf4`).

Everything else about using the viewer is in `RUNNING.md`, shipped beside this
file.

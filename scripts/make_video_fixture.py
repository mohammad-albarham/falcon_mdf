"""Writes an MF4 carrying a video stream, which no published sample set has.

Video in MDF 4 is not a channel of pixels. It is a *synchronisation* channel —
`cn_type` 4 — whose samples index into a media stream, paired with an
attachment naming that stream. asammdf writes exactly that shape, and hard-codes
the attachment as external, so the `.avi` sits beside the `.mf4` rather than
inside it. A published "sample MF4 with video" would therefore have to be a
multi-file bundle of a real vehicle recording, which is why searching for one
turns up nothing: GitHub has no repository matching "mdf4 video", and none of
the four vendor sets `fetch_reference_files.sh` pulls from contains one.

So this writes one. asammdf is the writer, which is the point: the file is
produced by an implementation independent of this crate, the same reasoning
`tests/write_conformance.rs` uses in the other direction.

What it cannot do is verify *values* — asammdf writing and asammdf checking
would be circular. There are no values to verify here. The question this
fixture answers is structural: given a channel this build deliberately does not
decode, does the file still open, does the master still read, is the refusal
explained, and does the attachment survive? `tests/sync_channel.rs` asserts
exactly that and nothing more.

    .venv/bin/python scripts/make_video_fixture.py [output.mf4]

Defaults to `test_data/generated/video_sync.mf4`, which is gitignored — no
measurement file is ever committed to this repository. Open that one in the GUI
to see how a media channel presents.
"""

import pathlib
import shutil
import subprocess
import sys

import numpy as np
from asammdf import MDF, Signal

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT = ROOT / "test_data" / "generated" / "video_sync.mf4"

FRAMES = 10
FRAME_INTERVAL = 0.04  # 25 fps, so the timestamps read like a real recording.


def write_video(path):
    """Writes the stream the MF4 points at, and returns its bytes.

    A real playable file where ffmpeg is available, so the fixture can actually
    be watched — one frame per sample of the sync channel, which is the
    relationship the channel exists to express. Without ffmpeg it falls back to
    a RIFF header, enough for the structure to be honest but not playable; the
    test asserts neither, so the fallback costs nothing but the viewing.
    """
    if shutil.which("ffmpeg"):
        subprocess.run(
            [
                "ffmpeg", "-y", "-loglevel", "error",
                "-f", "lavfi",
                "-i", f"testsrc=size=320x240:rate=25:duration={FRAMES * FRAME_INTERVAL}",
                "-c:v", "mjpeg", "-q:v", "5",
                str(path),
            ],
            check=True,
        )
        return path.read_bytes(), True

    stub = b"RIFF" + (2048).to_bytes(4, "little") + b"AVI LIST" + b"\x00" * 200
    path.write_bytes(stub)
    return stub, False


def main():
    out = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT
    out.parent.mkdir(parents=True, exist_ok=True)

    # The samples are frame indices into the stream, not measurements — which
    # is the whole reason a reader must not treat them as data.
    frames = Signal(
        samples=np.arange(FRAMES, dtype="<u8"),
        timestamps=np.arange(FRAMES, dtype=float) * FRAME_INTERVAL,
        name="VideoFrames",
        unit="",
        comment="frame index into the attached video stream",
    )
    frames.flags = Signal.Flags.stream_sync

    # The stream lands *beside* the MF4, which is where a real recording keeps
    # it: asammdf writes this attachment external, so the file names the video
    # rather than carrying it. Writing it here means the reference resolves and
    # the video can be opened in any player.
    video_path = out.parent / "drive.avi"
    avi, playable = write_video(video_path)
    frames.attachment = (avi, pathlib.Path(video_path.name), None)

    mdf = MDF(version="4.10")
    mdf.append([frames], comment="synthetic video-sync example")
    mdf.save(out, overwrite=True)

    # Report what was actually written rather than what was intended: if a
    # future asammdf stops emitting cn_type 4 here, this says so at once.
    check = MDF(out)
    channels = [
        (c.name, c.channel_type)
        for g in check.groups
        for c in g.channels
    ]
    attachments = [(a.file_name, a.mime) for a in check.attachments]
    print(f"wrote {out} ({out.stat().st_size} bytes)")
    print(f"  channels:    {channels}")
    print(f"  attachments: {attachments}")
    print(f"  stream:      {video_path} ({video_path.stat().st_size} bytes)")
    if playable:
        print(f"               {FRAMES} frames, one per sample — open it in any player")
    else:
        print("               ffmpeg not found, so this is a header stub and will not play")


if __name__ == "__main__":
    main()

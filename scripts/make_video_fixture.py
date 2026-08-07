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
import sys

import numpy as np
from asammdf import MDF, Signal

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT = ROOT / "test_data" / "generated" / "video_sync.mf4"

FRAMES = 10
FRAME_INTERVAL = 0.04  # 25 fps, so the timestamps read like a real recording.


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

    # A RIFF/AVI header, so the attachment is a plausible video rather than
    # arbitrary bytes. asammdf sets the MIME type itself; it is not ours to
    # choose here, and the test asserts what it chose.
    avi = b"RIFF" + (2048).to_bytes(4, "little") + b"AVI LIST" + b"\x00" * 200
    frames.attachment = (avi, pathlib.Path("drive.avi"), None)

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


if __name__ == "__main__":
    main()

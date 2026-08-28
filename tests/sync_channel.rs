//! A video stream, read from a file this crate did not write.
//!
//! MDF 4 stores video as a synchronisation channel — `cn_type` 4 — whose
//! samples index a media stream, plus an attachment naming that stream.
//! `scripts/make_video_fixture.py` writes one with asammdf, an implementation
//! independent of this crate, and this test reads it back, verifying that
//! sample indices and attachments decode correctly.
//!
//! Skips where the `.venv` or asammdf is absent, the same arrangement
//! `write_conformance.rs` uses.

use std::path::PathBuf;
use std::process::Command;

use falcon_mdf::blocks::ChannelType;
use falcon_mdf::Mf4File;

fn venv_python() -> Option<PathBuf> {
    let python = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python");
    python.is_file().then_some(python)
}

fn asammdf_available(python: &PathBuf) -> bool {
    Command::new(python)
        .args(["-c", "import asammdf"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn a_video_stream_sync_channel_decodes_its_samples_and_preserves_attachment() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: no .venv/bin/python");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping: asammdf not installed in the .venv");
        return;
    }

    // Written to a temp path rather than the script's default, so running the
    // suite neither depends on nor disturbs whatever is in `test_data/`.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("video_sync.mf4");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/make_video_fixture.py");
    if !script.is_file() {
        eprintln!("skipping: scripts/make_video_fixture.py absent");
        return;
    }

    let output = Command::new(&python)
        .arg(&script)
        .arg(&path)
        .output()
        .expect("failed to run the .venv python");
    assert!(
        output.status.success(),
        "the fixture generator failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let file = Mf4File::open(&path).expect("a file with a media channel must still open");

    let channels: Vec<_> = file.channels().cloned().collect();
    let video = channels
        .iter()
        .find(|c| c.name == "VideoFrames")
        .expect("the video channel should be listed, not hidden");

    // Readable and decoded. The samples are frame indices into the attached
    // stream, matching what asammdf produces.
    assert_eq!(video.unreadable(), None);
    assert_eq!(video.channel_type, ChannelType::Sync);

    let frames = file
        .signal(video)
        .and_then(|s| s.values_f64())
        .expect("a sync channel's samples decode to its frame indices");
    assert_eq!(frames, (0..10).map(|i| i as f64).collect::<Vec<f64>>());

    let chunk_frames: Vec<f64> = file
        .signal_chunks(video)
        .expect("signal_chunks should succeed for sync channel")
        .flat_map(|c| c.unwrap().values_f64().unwrap())
        .collect();
    assert_eq!(chunk_frames, (0..10).map(|i| i as f64).collect::<Vec<f64>>());

    // The rest of the file is unaffected: a refused channel must not cost the
    // reader the group it sits in.
    let master = file
        .master_channel(video.data_group_index, video.channel_group_index)
        .expect("the group's master channel");
    let times = file
        .signal(master)
        .and_then(|s| s.values_f64())
        .expect("the master must read even though its group holds a media channel");
    assert_eq!(times.len(), 10, "ten frames were written");
    assert!(
        (times[1] - 0.04).abs() < 1e-9,
        "frames are 25 fps apart, got {}",
        times[1]
    );

    // The stream itself. This is the only place the file says it is video at
    // all, so losing it would leave the channel unexplainable.
    let attachments = file.attachments();
    assert_eq!(attachments.len(), 1, "one attached stream");
    let at = &attachments[0];
    assert_eq!(at.file_name, "drive.avi");
    assert_eq!(
        at.file_path, "video/x-msvideo",
        "this field holds the attachment's MIME type, not a path"
    );
    assert!(
        !at.is_embedded,
        "asammdf writes the stream beside the file, not inside it"
    );
}

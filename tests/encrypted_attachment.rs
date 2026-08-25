//! Tests reading AES256-encrypted embedded attachments, cross-checked against asammdf.

use std::path::PathBuf;
use std::process::Command;

use falcon_mdf::{Mf4Error, Mf4File};

fn venv_python() -> Option<PathBuf> {
    let python = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../falcon_mdf/.venv/bin/python");
    if python.is_file() {
        return Some(python);
    }
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python");
    local.is_file().then_some(local)
}

fn asammdf_available(python: &PathBuf) -> bool {
    Command::new(python)
        .args(["-c", "import asammdf, cryptography"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn encrypted_attachments_are_decrypted_with_password_matching_asammdf() {
    let Some(python) = venv_python() else {
        eprintln!("skipping: no .venv/bin/python");
        return;
    };
    if !asammdf_available(&python) {
        eprintln!("skipping: asammdf or cryptography not installed in the .venv");
        return;
    }

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("encrypted_at.mf4");

    // Generate MF4 with asammdf containing encrypted attachments (both compressed and uncompressed)
    let script = r#"
import sys
from asammdf import MDF, Signal
import numpy as np

m = MDF()
t = np.array([0.0, 0.1, 0.2])
sig = Signal(samples=np.array([10, 20, 30]), timestamps=t, name='TestSig')
m.append(sig)

data1 = b"Top Secret Embedded Attachment 1: " + b"A" * 200
data2 = b"Top Secret Embedded Attachment 2: " + b"B" * 50

m.attach(data1, file_name='secret1.bin', comment='Encrypted compressed', compression=True, password='mypassword123')
m.attach(data2, file_name='secret2.bin', comment='Encrypted uncompressed', compression=False, password='another_password_that_is_quite_long_and_exceeds_32_bytes_total')

m.save(sys.argv[1], overwrite=True)
"#;

    let output = Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(&path)
        .output()
        .expect("run python script");
    assert!(
        output.status.success(),
        "python failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let file = Mf4File::open(&path).expect("open file");
    let attachments = file.attachments();
    assert_eq!(attachments.len(), 2, "two attachments created");

    // First attachment: compressed + encrypted with 'mypassword123'
    let at1 = &attachments[0];
    assert_eq!(at1.file_name, "secret1.bin");
    assert!(at1.is_embedded);
    assert!(at1.is_compressed);
    assert!(at1.is_encrypted());

    let enc1 = at1.encryption_info().expect("encryption info for at1");
    assert!(enc1.encrypted);
    assert_eq!(enc1.algorithm.to_ascii_lowercase(), "aes256");
    assert_eq!(enc1.original_size, 34 + 200);

    // Reading without password must fail with an Unsupported error explaining why
    let no_pass = file.attachment_data(at1);
    match no_pass {
        Err(Mf4Error::Unsupported { feature, detail }) => {
            assert!(feature.contains("encrypted attachment"));
            assert!(detail.contains("password must be provided"));
        }
        other => panic!("expected password required error, got: {other:?}"),
    }

    // Reading with correct password decrypts exact plaintext
    let decrypted1 = file
        .attachment_data_with_password(at1, Some("mypassword123"))
        .expect("decryption should succeed")
        .expect("embedded data exists");
    let expected1 = [b"Top Secret Embedded Attachment 1: ".as_slice(), &vec![b'A'; 200]].concat();
    assert_eq!(decrypted1, expected1);

    // Second attachment: uncompressed + encrypted with long password
    let at2 = &attachments[1];
    assert_eq!(at2.file_name, "secret2.bin");
    assert!(at2.is_embedded);
    assert!(!at2.is_compressed);
    assert!(at2.is_encrypted());

    let enc2 = at2.encryption_info().expect("encryption info for at2");
    assert!(enc2.encrypted);
    assert_eq!(enc2.algorithm.to_ascii_lowercase(), "aes256");
    assert_eq!(enc2.original_size, 34 + 50);

    let decrypted2 = file
        .attachment_data_with_password(
            at2,
            Some("another_password_that_is_quite_long_and_exceeds_32_bytes_total"),
        )
        .expect("decryption should succeed")
        .expect("embedded data exists");
    let expected2 = [b"Top Secret Embedded Attachment 2: ".as_slice(), &vec![b'B'; 50]].concat();
    assert_eq!(decrypted2, expected2);
}

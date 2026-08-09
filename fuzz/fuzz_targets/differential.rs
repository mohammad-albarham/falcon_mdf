//! Differential fuzz target: catches silent release-mode corruption.
//!
//! This target parses the same MF4 input twice — once in-process (debug
//! assertions on, the cargo-fuzz default) and once via a release-built helper
//! binary (debug assertions off, overflow checks off) — and panics if the
//! two decoded outputs differ. That catches bugs where a malformed file
//! produces a wrong-but-non-panicking result in true release builds while
//! the fuzzer's default pass/fail signal (panic vs no-panic) stays silent.
//!
//! Build the helper first:
//!
//! ```text
//! cargo +nightly fuzz build helper
//! ```
//!
//! Then run:
//!
//! ```text
//! HELPER_PATH=$(cargo +nightly fuzz build helper --message-format=short | tail -1)/helper cargo +nightly fuzz run differential
//! ```
//!
//! Or, if you built manually:
//!
//! ```text
//! HELPER_PATH=fuzz/target/release/helper cargo +nightly fuzz run differential
//! ```

#![no_main]
use libfuzzer_sys::fuzz_target;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use falcon_mdf::{Mf4File, SignalValues};

fuzz_target!(|data: &[u8]| {
    let helper_path = match env::var("HELPER_PATH") {
        Ok(p) => PathBuf::from(p),
        Err(_) => return,
    };
    if !helper_path.exists() {
        return;
    }

    let path = std::env::temp_dir().join(format!("falcon_mdf_diff_{}.mf4", std::process::id()));
    if fs::write(&path, data).is_err() {
        return;
    }

    let in_process = serialize_in_process(&path);
    let helper = serialize_helper(&path, &helper_path);

    let _ = fs::remove_file(&path);

    let (Some(a), Some(b)) = (in_process, helper) else {
        return;
    };
    if a != b {
        panic!("differential divergence: in-process and helper produced different outputs");
    }
});

fn serialize_in_process(path: &std::path::Path) -> Option<Vec<u8>> {
    let file = match Mf4File::open(path) {
        Ok(f) => f,
        Err(e) => return Some(encode_open_err(&e)),
    };

    let mut out = Vec::new();
    for ch in file.channels() {
        let name = ch.name.as_bytes();
        let name_len = name.len() as u32;

        let signal = match file.signal(ch) {
            Ok(s) => s,
            Err(e) => {
                write_u32(&mut out, name_len);
                out.write_all(name).unwrap();
                write_u8(&mut out, 1);
                write_string(&mut out, &e.to_string());
                continue;
            }
        };

        let values = match signal.values() {
            Ok(v) => v,
            Err(e) => {
                write_u32(&mut out, name_len);
                out.write_all(name).unwrap();
                write_u8(&mut out, 2);
                write_string(&mut out, &e.to_string());
                continue;
            }
        };

        let validity = signal.validity();

        write_u32(&mut out, name_len);
        out.write_all(name).unwrap();
        write_u8(&mut out, 0);
        serialize_signal_values(&mut out, &values);

        match validity {
            Some(v) => {
                let vcount = v.len() as u32;
                write_u32(&mut out, vcount);
                for &b in &v {
                    write_u8(&mut out, if b { 1 } else { 0 });
                }
            }
            None => {
                write_u32(&mut out, 0);
            }
        }
    }
    Some(out)
}

fn serialize_helper(path: &std::path::Path, helper: &std::path::Path) -> Option<Vec<u8>> {
    use std::process::Command;

    let output = match Command::new(helper).arg(path).output() {
        Ok(o) => o.stdout,
        Err(_) => return None,
    };
    Some(output)
}

fn encode_open_err(e: &falcon_mdf::Mf4Error) -> Vec<u8> {
    let mut out = Vec::new();
    write_u8(&mut out, 255);
    write_string(&mut out, &e.to_string());
    out
}

fn serialize_signal_values<W: Write>(w: &mut W, values: &SignalValues) {
    match values {
        SignalValues::U8(v) => {
            write_u8(w, 0);
            write_u32(w, v.len() as u32);
            w.write_all(v).unwrap();
        }
        SignalValues::U16(v) => {
            write_u8(w, 1);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u16(w, x);
            }
        }
        SignalValues::U32(v) => {
            write_u8(w, 2);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u32(w, x);
            }
        }
        SignalValues::U64(v) => {
            write_u8(w, 3);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u64(w, x);
            }
        }
        SignalValues::I8(v) => {
            write_u8(w, 4);
            write_u32(w, v.len() as u32);
            w.write_all(&v.iter().map(|&x| x as u8).collect::<Vec<u8>>()).unwrap();
        }
        SignalValues::I16(v) => {
            write_u8(w, 5);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u16(w, x as u16);
            }
        }
        SignalValues::I32(v) => {
            write_u8(w, 6);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u32(w, x as u32);
            }
        }
        SignalValues::I64(v) => {
            write_u8(w, 7);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u64(w, x as u64);
            }
        }
        SignalValues::F32(v) => {
            write_u8(w, 8);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u32(w, x.to_bits());
            }
        }
        SignalValues::F64(v) => {
            write_u8(w, 9);
            write_u32(w, v.len() as u32);
            for &x in v {
                write_u64(w, x.to_bits());
            }
        }
        SignalValues::Bytes { data, width } => {
            write_u8(w, 10);
            write_u64(w, *width as u64);
            write_u32(w, data.len() as u32);
            w.write_all(data).unwrap();
        }
        SignalValues::VarBytes { data, starts } => {
            write_u8(w, 11);
            write_u32(w, data.len() as u32);
            w.write_all(data).unwrap();
            write_u32(w, starts.len() as u32);
            for &s in starts {
                write_usize(w, s);
            }
        }
        SignalValues::Str(v) => {
            write_u8(w, 12);
            write_u32(w, v.len() as u32);
            for s in v {
                write_string(w, s);
            }
        }
        SignalValues::Complex { re, im } => {
            write_u8(w, 13);
            write_u32(w, re.len() as u32);
            for (&r, &i) in re.iter().zip(im.iter()) {
                write_u64(w, r.to_bits());
                write_u64(w, i.to_bits());
            }
        }
        SignalValues::CanopenDate(v) => {
            write_u8(w, 14);
            write_u32(w, v.len() as u32);
            for d in v {
                write_u16(w, d.year);
                write_u8(w, d.month);
                write_u8(w, d.day);
                write_u8(w, d.hour);
                write_u8(w, d.minute);
                write_u16(w, d.ms);
                write_u8(w, d.day_of_week);
                write_u8(w, if d.summer_time { 1 } else { 0 });
            }
        }
        SignalValues::CanopenTime(v) => {
            write_u8(w, 15);
            write_u32(w, v.len() as u32);
            for t in v {
                write_u32(w, t.ms_since_midnight);
                write_u16(w, t.days_since_1984);
            }
        }
        SignalValues::Array { values, elements_per_sample } => {
            write_u8(w, 16);
            write_u64(w, *elements_per_sample as u64);
            write_u32(w, values.len() as u32);
            for &x in values {
                write_u64(w, x.to_bits());
            }
        }
        SignalValues::ArrayVarLen { values, starts } => {
            write_u8(w, 17);
            write_u32(w, starts.len() as u32);
            for &s in starts {
                write_usize(w, s);
            }
            write_u32(w, values.len() as u32);
            for &x in values {
                write_u64(w, x.to_bits());
            }
        }
        _ => todo!(),
    }
}

fn write_u32<W: Write>(w: &mut W, v: u32) {
    w.write_all(&v.to_le_bytes()).unwrap();
}

fn write_u64<W: Write>(w: &mut W, v: u64) {
    w.write_all(&v.to_le_bytes()).unwrap();
}

fn write_u16<W: Write>(w: &mut W, v: u16) {
    w.write_all(&v.to_le_bytes()).unwrap();
}

fn write_u8<W: Write>(w: &mut W, v: u8) {
    w.write_all(&[v]).unwrap();
}

fn write_usize<W: Write>(w: &mut W, v: usize) {
    write_u64(w, v as u64);
}

fn write_string<W: Write>(w: &mut W, s: &str) {
    let bytes = s.as_bytes();
    write_u32(w, bytes.len() as u32);
    w.write_all(bytes).unwrap();
}

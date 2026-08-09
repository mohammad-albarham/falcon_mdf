use std::env;
use std::io::Write;
use std::process;

use falcon_mdf::{Mf4File, SignalValues};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.mf4>", args[0]);
        process::exit(1);
    }
    let path = &args[1];

    let file = match Mf4File::open(path) {
        Ok(f) => f,
        Err(e) => {
            let mut out = Vec::new();
            encode_open_err(&mut out, &e);
            write_output(&out);
            return;
        }
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

    write_output(&out);
}

fn write_output(data: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(data).unwrap();
    stdout.flush().unwrap();
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

fn encode_open_err<W: Write>(w: &mut W, e: &falcon_mdf::Mf4Error) {
    write_u8(w, 255);
    write_string(w, &e.to_string());
}

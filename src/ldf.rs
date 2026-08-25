//! Reading a LIN Description File (LDF) database into a [`CanDatabase`].
//!
//! LDF is the standard database format for the LIN (Local Interconnect Network)
//! bus. LIN frames are logged into MF4 files under `LIN_Frame.ID` and
//! `LIN_Frame.DataBytes`, with 6-bit frame identifiers (0..=63) and payloads up to
//! 8 bytes.
//!
//! This module parses `.ldf` files and converts their frames, signals, physical
//! encodings and logical value tables into the front-end-neutral definitions in
//! [`crate::candb`], allowing LIN frames to be decoded into physical signals and
//! named text labels using [`crate::Mf4File::decode_lin`] or [`CanDatabase::decode`].

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::candb::{CanDatabase, MessageDef, Multiplexing, SignalDef};
use crate::error::{Mf4Error, Result};

impl CanDatabase {
    /// Loads a database from LIN Description File (LDF) bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Mf4Error::ParseError`] when the LDF syntax is invalid or
    /// malformed.
    pub fn from_ldf(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes).map_err(|e| {
            Mf4Error::parse_error(format!("LDF content is not valid UTF-8: {e}"))
        })?;
        Self::from_ldf_str(text)
    }

    /// Loads a database from an LDF string.
    ///
    /// # Errors
    ///
    /// Returns [`Mf4Error::ParseError`] when the LDF syntax is invalid or
    /// malformed.
    pub fn from_ldf_str(text: &str) -> Result<Self> {
        parse_ldf(text)
    }

    /// Loads a database from an LDF file on disk.
    ///
    /// # Errors
    ///
    /// Returns [`Mf4Error::Io`] on read failure, or [`Mf4Error::ParseError`] on
    /// parse failure.
    pub fn from_ldf_path(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = fs::read(path.as_ref())?;
        Self::from_ldf(&bytes)
    }
}

/// Tokenizer for LDF files.
#[derive(Debug, Clone, PartialEq)]
enum Token<'a> {
    Ident(&'a str),
    StringLit(&'a str),
    Int(i64),
    Float(f64),
    Colon,
    SemiColon,
    Comma,
    Equals,
    BraceOpen,
    BraceClose,
    BracketOpen,
    BracketClose,
}

struct Lexer<'a> {
    input: &'a str,
    chars: std::str::CharIndices<'a>,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Lexer {
            input,
            chars: input.char_indices(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.clone().next().map(|(_, c)| c)
    }

    fn next_token(&mut self) -> Result<Option<Token<'a>>> {
        loop {
            let Some((start, ch)) = self.chars.next() else {
                return Ok(None);
            };

            match ch {
                ' ' | '\t' | '\r' | '\n' => continue,
                '/' => {
                    if let Some('/') = self.peek() {
                        self.chars.next();
                        for (_, c) in self.chars.by_ref() {
                            if c == '\n' {
                                break;
                            }
                        }
                        continue;
                    } else if let Some('*') = self.peek() {
                        self.chars.next();
                        let mut prev = ' ';
                        for (_, c) in self.chars.by_ref() {
                            if prev == '*' && c == '/' {
                                break;
                            }
                            prev = c;
                        }
                        continue;
                    } else {
                        return Err(Mf4Error::parse_error(format!(
                            "unexpected character '/' at offset {start}"
                        )));
                    }
                }
                ':' => return Ok(Some(Token::Colon)),
                ';' => return Ok(Some(Token::SemiColon)),
                ',' => return Ok(Some(Token::Comma)),
                '=' => return Ok(Some(Token::Equals)),
                '{' => return Ok(Some(Token::BraceOpen)),
                '}' => return Ok(Some(Token::BraceClose)),
                '[' => return Ok(Some(Token::BracketOpen)),
                ']' => return Ok(Some(Token::BracketClose)),
                '"' => {
                    let str_start = start + 1;
                    let mut end = str_start;
                    for (i, c) in self.chars.by_ref() {
                        if c == '"' {
                            end = i;
                            break;
                        }
                    }
                    return Ok(Some(Token::StringLit(&self.input[str_start..end])));
                }
                _ if ch.is_ascii_alphabetic() || ch == '_' => {
                    let mut end = start + ch.len_utf8();
                    while let Some((i, next_c)) = self.chars.clone().next() {
                        if next_c.is_ascii_alphanumeric() || next_c == '_' || next_c == '-' {
                            self.chars.next();
                            end = i + next_c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    return Ok(Some(Token::Ident(&self.input[start..end])));
                }
                _ if ch.is_ascii_digit() || (ch == '-' && self.peek().is_some_and(|c| c.is_ascii_digit())) => {
                    let mut end = start + ch.len_utf8();
                    let mut is_hex = false;
                    let mut is_float = false;

                    if ch == '0' && (self.peek() == Some('x') || self.peek() == Some('X')) {
                        is_hex = true;
                        self.chars.next(); // consume 'x'
                        end += 1;
                    }

                    while let Some((i, next_c)) = self.chars.clone().next() {
                        let matches_hex = is_hex && next_c.is_ascii_hexdigit();
                        let matches_dec = !is_hex && next_c.is_ascii_digit();

                        if matches_hex || matches_dec {
                            self.chars.next();
                            end = i + next_c.len_utf8();
                        } else if !is_hex && next_c == '.' && !is_float {
                            // Check if next char after '.' is digit
                            let mut clone = self.chars.clone();
                            clone.next();
                            if clone.next().is_some_and(|(_, c)| c.is_ascii_digit()) {
                                is_float = true;
                                self.chars.next();
                                end = i + next_c.len_utf8();
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }

                    let raw = &self.input[start..end];
                    if is_hex {
                        let hex_str = raw.trim_start_matches("0x").trim_start_matches("0X");
                        let val = i64::from_str_radix(hex_str, 16).map_err(|e| {
                            Mf4Error::parse_error(format!("invalid hex integer '{raw}': {e}"))
                        })?;
                        return Ok(Some(Token::Int(val)));
                    } else if is_float {
                        let val: f64 = raw.parse().map_err(|e| {
                            Mf4Error::parse_error(format!("invalid float '{raw}': {e}"))
                        })?;
                        return Ok(Some(Token::Float(val)));
                    } else {
                        let val: i64 = raw.parse().map_err(|e| {
                            Mf4Error::parse_error(format!("invalid integer '{raw}': {e}"))
                        })?;
                        return Ok(Some(Token::Int(val)));
                    }
                }
                _ => {
                    return Err(Mf4Error::parse_error(format!(
                        "unexpected character '{ch}' at offset {start}"
                    )));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct EncodingDef {
    factor: f64,
    offset: f64,
    unit: String,
    signed: bool,
    logical_values: Vec<(i64, String)>,
}

#[derive(Debug, Clone)]
struct RawFrame {
    name: String,
    id: u32,
    length: u64,
    signals: Vec<(String, u64)>,
}

fn parse_ldf(input: &str) -> Result<CanDatabase> {
    let mut lexer = Lexer::new(input);
    let mut tokens = Vec::new();
    while let Some(tok) = lexer.next_token()? {
        tokens.push(tok);
    }

    let mut cursor = 0usize;
    let mut signals: HashMap<String, u64> = HashMap::new();
    let mut encodings: HashMap<String, EncodingDef> = HashMap::new();
    let mut signal_representations: HashMap<String, String> = HashMap::new();
    let mut frames_raw: Vec<RawFrame> = Vec::new();

    while cursor < tokens.len() {
        match &tokens[cursor] {
            Token::Ident("Signals") => {
                cursor += 1;
                parse_signals_section(&tokens, &mut cursor, &mut signals)?;
            }
            Token::Ident("Diagnostic_signals") => {
                cursor += 1;
                parse_diagnostic_signals_section(&tokens, &mut cursor, &mut signals)?;
            }
            Token::Ident("Frames") => {
                cursor += 1;
                parse_frames_section(&tokens, &mut cursor, &mut frames_raw)?;
            }
            Token::Ident("Diagnostic_frames") => {
                cursor += 1;
                parse_diagnostic_frames_section(&tokens, &mut cursor, &mut frames_raw)?;
            }
            Token::Ident("Signal_encoding_types") => {
                cursor += 1;
                parse_signal_encoding_types(&tokens, &mut cursor, &mut encodings)?;
            }
            Token::Ident("Signal_representation") => {
                cursor += 1;
                parse_signal_representation(&tokens, &mut cursor, &mut signal_representations)?;
            }
            Token::Ident(_) => {
                cursor += 1;
                // Skip generic top-level statement or block
                skip_statement_or_block(&tokens, &mut cursor);
            }
            _ => {
                cursor += 1;
            }
        }
    }

    let mut messages = Vec::new();
    for frame in frames_raw {
        let mut msg_signals = Vec::new();
        for (sig_name, start_bit) in frame.signals {
            let size = signals.get(&sig_name).copied().unwrap_or(8);
            let encoding = signal_representations
                .get(&sig_name)
                .and_then(|enc_name| encodings.get(enc_name));

            let (factor, offset, unit, signed, value_table) = match encoding {
                Some(enc) => (
                    if enc.factor == 0.0 { 1.0 } else { enc.factor },
                    enc.offset,
                    enc.unit.clone(),
                    enc.signed,
                    enc.logical_values.clone(),
                ),
                None => (1.0, 0.0, String::new(), false, Vec::new()),
            };

            msg_signals.push(SignalDef {
                name: sig_name,
                start_bit,
                size,
                big_endian: false,
                signed,
                factor,
                offset,
                unit,
                multiplexing: Multiplexing::None,
                value_table,
            });
        }

        messages.push(MessageDef {
            name: frame.name,
            id: frame.id & 0x3F,
            extended: false,
            length: frame.length,
            signals: msg_signals,
        });
    }

    Ok(CanDatabase::new(messages))
}

fn skip_statement_or_block(tokens: &[Token<'_>], cursor: &mut usize) {
    let mut brace_depth = 0;
    while *cursor < tokens.len() {
        match &tokens[*cursor] {
            Token::BraceOpen => {
                brace_depth += 1;
                *cursor += 1;
            }
            Token::BraceClose => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                    *cursor += 1;
                    if brace_depth == 0 {
                        break;
                    }
                } else {
                    *cursor += 1;
                    break;
                }
            }
            Token::SemiColon if brace_depth == 0 => {
                *cursor += 1;
                break;
            }
            _ => {
                *cursor += 1;
            }
        }
    }
}

fn parse_signals_section(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    signals: &mut HashMap<String, u64>,
) -> Result<()> {
    if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
        return Err(Mf4Error::parse_error("expected '{' after Signals"));
    }
    *cursor += 1;

    while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
        // SignalName : size , init_val , publisher , subscribers... ;
        let Token::Ident(sig_name) = tokens[*cursor] else {
            *cursor += 1;
            continue;
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::Colon {
            return Err(Mf4Error::parse_error(format!(
                "expected ':' after signal name '{sig_name}'"
            )));
        }
        *cursor += 1;

        let size = match tokens.get(*cursor) {
            Some(Token::Int(sz)) => *sz as u64,
            _ => {
                return Err(Mf4Error::parse_error(format!(
                    "expected integer size for signal '{sig_name}'"
                )));
            }
        };
        signals.insert(sig_name.to_string(), size);
        *cursor += 1;

        // Skip to semicolon
        while *cursor < tokens.len() && tokens[*cursor] != Token::SemiColon {
            *cursor += 1;
        }
        if *cursor < tokens.len() && tokens[*cursor] == Token::SemiColon {
            *cursor += 1;
        }
    }

    if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
        *cursor += 1;
    }
    Ok(())
}

fn parse_diagnostic_signals_section(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    signals: &mut HashMap<String, u64>,
) -> Result<()> {
    if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
        return Err(Mf4Error::parse_error("expected '{' after Diagnostic_signals"));
    }
    *cursor += 1;

    while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
        let Token::Ident(sig_name) = tokens[*cursor] else {
            *cursor += 1;
            continue;
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::Colon {
            return Err(Mf4Error::parse_error(format!(
                "expected ':' after diagnostic signal '{sig_name}'"
            )));
        }
        *cursor += 1;

        let size = match tokens.get(*cursor) {
            Some(Token::Int(sz)) => *sz as u64,
            _ => 8,
        };
        signals.insert(sig_name.to_string(), size);
        *cursor += 1;

        while *cursor < tokens.len() && tokens[*cursor] != Token::SemiColon {
            *cursor += 1;
        }
        if *cursor < tokens.len() && tokens[*cursor] == Token::SemiColon {
            *cursor += 1;
        }
    }

    if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
        *cursor += 1;
    }
    Ok(())
}

fn parse_frames_section(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    frames: &mut Vec<RawFrame>,
) -> Result<()> {
    if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
        return Err(Mf4Error::parse_error("expected '{' after Frames"));
    }
    *cursor += 1;

    while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
        // FrameName: id, publisher, length { sig1, offset1; sig2, offset2; }
        let Token::Ident(frame_name) = tokens[*cursor] else {
            *cursor += 1;
            continue;
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::Colon {
            return Err(Mf4Error::parse_error(format!(
                "expected ':' after frame name '{frame_name}'"
            )));
        }
        *cursor += 1;

        let frame_id = match tokens.get(*cursor) {
            Some(Token::Int(id)) => *id as u32,
            _ => {
                return Err(Mf4Error::parse_error(format!(
                    "expected integer id for frame '{frame_name}'"
                )));
            }
        };
        *cursor += 1;

        if *cursor < tokens.len() && tokens[*cursor] == Token::Comma {
            *cursor += 1;
        }
        // publisher node (ident)
        if *cursor < tokens.len() && matches!(tokens[*cursor], Token::Ident(_)) {
            *cursor += 1;
        }
        if *cursor < tokens.len() && tokens[*cursor] == Token::Comma {
            *cursor += 1;
        }

        let frame_length = match tokens.get(*cursor) {
            Some(Token::Int(len)) => *len as u64,
            _ => 8,
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
            return Err(Mf4Error::parse_error(format!(
                "expected '{{' for frame signals in '{frame_name}'"
            )));
        }
        *cursor += 1;

        let mut frame_signals = Vec::new();
        while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
            let Token::Ident(sig_name) = tokens[*cursor] else {
                *cursor += 1;
                continue;
            };
            *cursor += 1;

            if *cursor < tokens.len() && tokens[*cursor] == Token::Comma {
                *cursor += 1;
            }

            let start_bit = match tokens.get(*cursor) {
                Some(Token::Int(sb)) => *sb as u64,
                _ => 0,
            };
            *cursor += 1;

            if *cursor < tokens.len() && tokens[*cursor] == Token::SemiColon {
                *cursor += 1;
            }

            frame_signals.push((sig_name.to_string(), start_bit));
        }

        if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
            *cursor += 1;
        }

        frames.push(RawFrame {
            name: frame_name.to_string(),
            id: frame_id,
            length: frame_length,
            signals: frame_signals,
        });
    }

    if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
        *cursor += 1;
    }
    Ok(())
}

fn parse_diagnostic_frames_section(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    frames: &mut Vec<RawFrame>,
) -> Result<()> {
    if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
        return Err(Mf4Error::parse_error("expected '{' after Diagnostic_frames"));
    }
    *cursor += 1;

    while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
        let Token::Ident(frame_name) = tokens[*cursor] else {
            *cursor += 1;
            continue;
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::Colon {
            return Err(Mf4Error::parse_error(format!(
                "expected ':' after diagnostic frame '{frame_name}'"
            )));
        }
        *cursor += 1;

        let frame_id = match tokens.get(*cursor) {
            Some(Token::Int(id)) => *id as u32,
            _ => 60,
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
            return Err(Mf4Error::parse_error(format!(
                "expected '{{' for diagnostic frame '{frame_name}'"
            )));
        }
        *cursor += 1;

        let mut frame_signals = Vec::new();
        while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
            let Token::Ident(sig_name) = tokens[*cursor] else {
                *cursor += 1;
                continue;
            };
            *cursor += 1;

            if *cursor < tokens.len() && tokens[*cursor] == Token::Comma {
                *cursor += 1;
            }

            let start_bit = match tokens.get(*cursor) {
                Some(Token::Int(sb)) => *sb as u64,
                _ => 0,
            };
            *cursor += 1;

            if *cursor < tokens.len() && tokens[*cursor] == Token::SemiColon {
                *cursor += 1;
            }

            frame_signals.push((sig_name.to_string(), start_bit));
        }

        if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
            *cursor += 1;
        }

        frames.push(RawFrame {
            name: frame_name.to_string(),
            id: frame_id,
            length: 8,
            signals: frame_signals,
        });
    }

    if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
        *cursor += 1;
    }
    Ok(())
}

fn parse_signal_encoding_types(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    encodings: &mut HashMap<String, EncodingDef>,
) -> Result<()> {
    if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
        return Err(Mf4Error::parse_error("expected '{' after Signal_encoding_types"));
    }
    *cursor += 1;

    while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
        let Token::Ident(enc_name) = tokens[*cursor] else {
            *cursor += 1;
            continue;
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
            return Err(Mf4Error::parse_error(format!(
                "expected '{{' for encoding type '{enc_name}'"
            )));
        }
        *cursor += 1;

        let mut enc = EncodingDef {
            factor: 1.0,
            offset: 0.0,
            unit: String::new(),
            signed: false,
            logical_values: Vec::new(),
        };

        while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
            match tokens.get(*cursor) {
                Some(Token::Ident("physical_value")) => {
                    *cursor += 1;
                    // physical_value, min, max, factor, offset, "unit" ;
                    let mut items = Vec::new();
                    while *cursor < tokens.len() && tokens[*cursor] != Token::SemiColon {
                        if tokens[*cursor] != Token::Comma {
                            items.push(&tokens[*cursor]);
                        }
                        *cursor += 1;
                    }
                    if *cursor < tokens.len() && tokens[*cursor] == Token::SemiColon {
                        *cursor += 1;
                    }

                    // items: [min, max, factor, offset, optional_unit]
                    if items.len() >= 4 {
                        if let Some(f) = token_to_f64(items[2]) {
                            enc.factor = f;
                        }
                        if let Some(o) = token_to_f64(items[3]) {
                            enc.offset = o;
                        }
                        if items.len() >= 5 {
                            match items[4] {
                                Token::StringLit(u) | Token::Ident(u) => {
                                    enc.unit = u.to_string();
                                }
                                _ => {}
                            }
                        }
                        // Check if min_raw is negative
                        if let Some(min_val) = token_to_f64(items[0]) {
                            if min_val < 0.0 {
                                enc.signed = true;
                            }
                        }
                    }
                }
                Some(Token::Ident("logical_value")) => {
                    *cursor += 1;
                    // logical_value, raw_value, "description" ;
                    let mut items = Vec::new();
                    while *cursor < tokens.len() && tokens[*cursor] != Token::SemiColon {
                        if tokens[*cursor] != Token::Comma {
                            items.push(&tokens[*cursor]);
                        }
                        *cursor += 1;
                    }
                    if *cursor < tokens.len() && tokens[*cursor] == Token::SemiColon {
                        *cursor += 1;
                    }

                    if items.len() >= 2 {
                        let raw_val = match items[0] {
                            Token::Int(v) => *v,
                            _ => 0,
                        };
                        let desc = match items[1] {
                            Token::StringLit(s) | Token::Ident(s) => s.to_string(),
                            _ => String::new(),
                        };
                        enc.logical_values.push((raw_val, desc));
                    }
                }
                _ => {
                    *cursor += 1;
                }
            }
        }

        if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
            *cursor += 1;
        }

        encodings.insert(enc_name.to_string(), enc);
    }

    if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
        *cursor += 1;
    }
    Ok(())
}

fn token_to_f64(token: &Token<'_>) -> Option<f64> {
    match token {
        Token::Float(f) => Some(*f),
        Token::Int(i) => Some(*i as f64),
        _ => None,
    }
}

fn parse_signal_representation(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    representations: &mut HashMap<String, String>,
) -> Result<()> {
    if *cursor >= tokens.len() || tokens[*cursor] != Token::BraceOpen {
        return Err(Mf4Error::parse_error("expected '{' after Signal_representation"));
    }
    *cursor += 1;

    while *cursor < tokens.len() && tokens[*cursor] != Token::BraceClose {
        // EncodingName: Signal1, Signal2, ... ;
        let Token::Ident(enc_name) = tokens[*cursor] else {
            *cursor += 1;
            continue;
        };
        *cursor += 1;

        if *cursor >= tokens.len() || tokens[*cursor] != Token::Colon {
            return Err(Mf4Error::parse_error(format!(
                "expected ':' after encoding name '{enc_name}'"
            )));
        }
        *cursor += 1;

        while *cursor < tokens.len() && tokens[*cursor] != Token::SemiColon {
            if let Token::Ident(sig_name) = tokens[*cursor] {
                representations.insert(sig_name.to_string(), enc_name.to_string());
            }
            *cursor += 1;
        }

        if *cursor < tokens.len() && tokens[*cursor] == Token::SemiColon {
            *cursor += 1;
        }
    }

    if *cursor < tokens.len() && tokens[*cursor] == Token::BraceClose {
        *cursor += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LDF: &str = r#"
LIN_description_file ;
LIN_protocol_version = "2.1" ;
LIN_language_version = "2.1" ;
LIN_speed = 19.2 kbps ;

Nodes {
    Master: CEM, 5.0 ms, 0.1 ms ;
    Slaves: LSM, RSM ;
}

Signals {
    LsmDoorState: 2, 0, LSM, CEM ;
    LsmMirrorAngle: 10, 512, LSM, CEM ;
    LsmTemp: 8, 40, LSM, CEM ;
}

Frames {
    LsmFrame: 33, LSM, 4 {
        LsmDoorState, 0 ;
        LsmMirrorAngle, 2 ;
        LsmTemp, 16 ;
    }
}

Signal_encoding_types {
    EncDoorState {
        logical_value, 0, "Closed" ;
        logical_value, 1, "Ajar" ;
        logical_value, 2, "Open" ;
        logical_value, 3, "Error" ;
    }
    EncMirrorAngle {
        physical_value, 0, 1023, 0.1, -50.0, "deg" ;
    }
    EncTemp {
        physical_value, 0, 255, 1.0, -40.0, "degC" ;
    }
}

Signal_representation {
    EncDoorState: LsmDoorState ;
    EncMirrorAngle: LsmMirrorAngle ;
    EncTemp: LsmTemp ;
}
"#;

    #[test]
    fn test_ldf_parsing_and_decoding() {
        let db = CanDatabase::from_ldf(SAMPLE_LDF.as_bytes()).expect("LDF must parse");
        assert_eq!(db.messages().len(), 1);

        let msg = db.message(33).expect("frame 33");
        assert_eq!(msg.name, "LsmFrame");
        assert_eq!(msg.signals.len(), 3);

        // Payload:
        // LsmDoorState = 2 (Open) -> bits 0..2 = 2
        // LsmMirrorAngle = 500 -> bits 2..12 = 500 (value = 500 * 0.1 - 50 = 0.0 deg)
        // Byte 0 = (2 | (500 << 2)) & 0xFF = (2 | 0xD0) = 0xD2
        // Byte 1 = (500 >> 6) & 0xFF = 7
        // LsmTemp = 65 (65 - 40 = 25.0 degC) -> bits 16..24 = byte 2 = 65
        let payload = [0xD2, 0x07, 65, 0x00];
        let decoded = db.decode(33, &payload);

        let door = decoded.iter().find(|s| s.name == "LsmDoorState").unwrap();
        assert_eq!(door.value, 2.0);
        assert_eq!(door.text, Some("Open"));

        let mirror = decoded.iter().find(|s| s.name == "LsmMirrorAngle").unwrap();
        assert_eq!(mirror.value, 0.0);
        assert_eq!(mirror.unit, "deg");

        let temp = decoded.iter().find(|s| s.name == "LsmTemp").unwrap();
        assert_eq!(temp.value, 25.0);
        assert_eq!(temp.unit, "degC");
    }
}

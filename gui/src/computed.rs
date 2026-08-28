//! Expression parser and evaluator for computed channels in the plot panel.
//!
//! A computed channel is an arithmetic expression over channels in the file
//! (e.g. `EngineSpeed * 0.10472 * Torque / 1000.0` or `[FL Speed] - [FR Speed]`).
//! It evaluates to a virtual [`ChannelSignal`] that can be plotted, decimated,
//! inspected with measurement cursors, and saved with the file session.

use std::collections::HashMap;
use std::sync::Arc;

use falcon_mdf::Mf4File;

use crate::model::ChannelLoc;
use crate::signal_loader::{decode_channel, ChannelSignal, SignalLoadResult};

/// Definition of a user-created computed channel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ComputedDef {
    pub name: String,
    pub expression: String,
    pub unit: String,
    /// Whether the definition is drawn — and evaluated. A hidden definition
    /// stays in the editor and costs nothing per frame; only visible ones
    /// are evaluated.
    pub visible: bool,
}

impl ComputedDef {
    pub fn new(
        name: impl Into<String>,
        expression: impl Into<String>,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            expression: expression.into(),
            unit: unit.into(),
            visible: true,
        }
    }
}

/// Abstract syntax tree for computed channel expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Channel(String),
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Returns the list of distinct channel names referenced in this expression.
    pub fn referenced_channels(&self) -> Vec<String> {
        let mut names = Vec::new();
        self.collect_channels(&mut names);
        names
    }

    fn collect_channels(&self, out: &mut Vec<String>) {
        match self {
            Expr::Number(_) => {}
            Expr::Channel(name) => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Expr::Neg(inner) => inner.collect_channels(out),
            Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) => {
                a.collect_channels(out);
                b.collect_channels(out);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

/// Lexes an expression string into a list of tokens.
fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '[' | '{' | '"' | '\'' => {
                let close = match c {
                    '[' => ']',
                    '{' => '}',
                    '"' => '"',
                    '\'' => '\'',
                    _ => unreachable!(),
                };
                i += 1;
                let start = i;
                while i < len && chars[i] != close {
                    i += 1;
                }
                if i >= len {
                    return Err(format!(
                        "unclosed quoted channel reference starting with '{c}'"
                    ));
                }
                let name: String = chars[start..i].iter().collect();
                let trimmed = name.trim().to_string();
                if trimmed.is_empty() {
                    return Err("empty channel reference inside brackets/quotes".to_string());
                }
                tokens.push(Token::Ident(trimmed));
                i += 1; // skip closing delimiter
            }
            '0'..='9' | '.' => {
                let start = i;
                let mut has_dot = c == '.';
                let mut has_exp = false;
                i += 1;
                while i < len {
                    let next = chars[i];
                    if next.is_ascii_digit() {
                        i += 1;
                    } else if next == '.' && !has_dot && !has_exp {
                        has_dot = true;
                        i += 1;
                    } else if (next == 'e' || next == 'E') && !has_exp {
                        has_exp = true;
                        i += 1;
                        if i < len && (chars[i] == '+' || chars[i] == '-') {
                            i += 1;
                        }
                    } else {
                        break;
                    }
                }
                let s: String = chars[start..i].iter().collect();
                if s == "." {
                    return Err("unexpected standalone dot '.'".to_string());
                }
                let val: f64 = s
                    .parse()
                    .map_err(|e| format!("invalid number literal '{s}': {e}"))?;
                tokens.push(Token::Number(val));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                i += 1;
                while i < len {
                    let next = chars[i];
                    if next.is_alphanumeric() || next == '_' || next == '.' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let name: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(name));
            }
            other => {
                return Err(format!("unexpected character '{other}' in expression"));
            }
        }
    }

    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

/// Maximum nesting depth for a computed expression.
///
/// The parser recurses once per parenthesis level and once per unary
/// operator, and the evaluator and the AST's drop recurse over the same
/// shape. Expressions arrive from pastes and from session files, which are
/// external input, so a bound turns nesting deep enough to overflow the
/// stack into an ordinary parse error instead of an abort; the core library
/// bounds composition nesting for the same reason (`MAX_COMPOSITION_DEPTH`).
/// Real expressions nest a handful of levels; 32 leaves wide margin.
pub const MAX_EXPRESSION_DEPTH: usize = 32;

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(tok)
    }

    /// Descends one nesting level, refusing to go past
    /// [`MAX_EXPRESSION_DEPTH`].
    fn deeper(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_EXPRESSION_DEPTH {
            return Err(format!(
                "expression nesting exceeds the maximum depth of {MAX_EXPRESSION_DEPTH}"
            ));
        }
        Ok(())
    }

    fn parse_expression(&mut self) -> Result<Expr, String> {
        self.parse_add_sub()
    }

    fn parse_add_sub(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul_div()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Plus => {
                    self.next();
                    let right = self.parse_mul_div()?;
                    left = Expr::Add(Box::new(left), Box::new(right));
                }
                Token::Minus => {
                    self.next();
                    let right = self.parse_mul_div()?;
                    left = Expr::Sub(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul_div(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Star => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = Expr::Mul(Box::new(left), Box::new(right));
                }
                Token::Slash => {
                    self.next();
                    let right = self.parse_unary()?;
                    left = Expr::Div(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if let Some(tok) = self.peek() {
            match tok {
                Token::Minus => {
                    self.next();
                    self.deeper()?;
                    let inner = self.parse_unary()?;
                    self.depth -= 1;
                    return Ok(Expr::Neg(Box::new(inner)));
                }
                Token::Plus => {
                    self.next();
                    self.deeper()?;
                    let inner = self.parse_unary()?;
                    self.depth -= 1;
                    return Ok(inner);
                }
                _ => {}
            }
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr, String> {
        let Some(tok) = self.next() else {
            return Err("unexpected end of expression".to_string());
        };
        match tok {
            Token::Number(n) => Ok(Expr::Number(*n)),
            Token::Ident(name) => Ok(Expr::Channel(name.clone())),
            Token::LParen => {
                self.deeper()?;
                let inner = self.parse_expression()?;
                self.depth -= 1;
                match self.next() {
                    Some(Token::RParen) => Ok(inner),
                    _ => Err("expected ')' to close parenthesis".to_string()),
                }
            }
            Token::Plus | Token::Minus | Token::Star | Token::Slash => {
                Err("unexpected binary operator".to_string())
            }
            Token::RParen => Err("unexpected ')'".to_string()),
        }
    }
}

/// Parses an arithmetic expression string into an [`Expr`] AST.
pub fn parse_expr(input: &str) -> Result<Expr, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("expression is empty".to_string());
    }
    let tokens = tokenize(trimmed)?;
    if tokens.is_empty() {
        return Err("expression is empty".to_string());
    }
    let mut parser = Parser::new(&tokens);
    let expr = parser.parse_expression()?;
    if parser.pos < tokens.len() {
        return Err("unexpected extra tokens after expression".to_string());
    }
    Ok(expr)
}

/// Resamples a source series onto a target timestamp vector using linear interpolation.
///
/// Returns interpolated values and updated validity mask.
pub fn resample_linear(
    src_times: &[f64],
    src_values: &[f64],
    src_valid: Option<&[bool]>,
    target_times: &[f64],
) -> (Vec<f64>, Option<Vec<bool>>) {
    let n_src = src_times.len().min(src_values.len());
    let n_target = target_times.len();

    if n_src == 0 || n_target == 0 {
        return (vec![f64::NAN; n_target], Some(vec![false; n_target]));
    }

    let src_times = &src_times[..n_src];
    let src_values = &src_values[..n_src];

    let mut out_values = Vec::with_capacity(n_target);
    let mut out_valid = Vec::with_capacity(n_target);
    let mut has_any_invalid = src_valid.is_some();

    for &t in target_times {
        if t < src_times[0] {
            // Before range: clamp to first sample
            out_values.push(src_values[0]);
            let v = src_valid
                .map(|v| v.first().copied().unwrap_or(true))
                .unwrap_or(true);
            out_valid.push(v);
            if !v {
                has_any_invalid = true;
            }
        } else if t >= src_times[n_src - 1] {
            // At or after last sample: clamp to last sample
            out_values.push(src_values[n_src - 1]);
            let v = src_valid
                .map(|v| v.get(n_src - 1).copied().unwrap_or(true))
                .unwrap_or(true);
            out_valid.push(v);
            if !v {
                has_any_invalid = true;
            }
        } else {
            // Binary search interval [i, i+1]
            let next_idx = src_times.partition_point(|&st| st <= t);
            let i = if next_idx == 0 { 0 } else { next_idx - 1 };
            let j = (i + 1).min(n_src - 1);

            let t0 = src_times[i];
            let t1 = src_times[j];
            let v0 = src_values[i];
            let v1 = src_values[j];

            let val = if (t1 - t0).abs() < 1e-12 {
                v0
            } else {
                let frac = (t - t0) / (t1 - t0);
                v0 + (v1 - v0) * frac
            };
            out_values.push(val);

            let valid0 = src_valid
                .map(|v| v.get(i).copied().unwrap_or(true))
                .unwrap_or(true);
            let valid1 = src_valid
                .map(|v| v.get(j).copied().unwrap_or(true))
                .unwrap_or(true);
            let is_valid = valid0 && valid1;
            out_valid.push(is_valid);
            if !is_valid {
                has_any_invalid = true;
            }
        }
    }

    let valid_opt = if has_any_invalid {
        Some(out_valid)
    } else {
        None
    };
    (out_values, valid_opt)
}

/// Evaluates an AST over pre-loaded channel signals.
///
/// Unifies timebases across operand channels via sorted timestamp union and linear
/// interpolation. Produces a single unified [`ChannelSignal`].
pub fn eval_expr(
    name: &str,
    unit: &str,
    expr: &Expr,
    signals: &HashMap<String, &ChannelSignal>,
) -> Result<ChannelSignal, String> {
    let req_channels = expr.referenced_channels();

    // Verify all referenced channels exist in the provided signals map
    for ch_name in &req_channels {
        if !signals.contains_key(ch_name) {
            return Err(format!(
                "referenced channel '{ch_name}' is not loaded or missing"
            ));
        }
    }

    if req_channels.is_empty() {
        // Pure constant expression
        let val = eval_scalar(expr)?;
        return Ok(ChannelSignal {
            loc: ChannelLoc {
                data_group_index: usize::MAX,
                channel_group_index: 0,
                channel_index: 0,
            },
            name: name.to_string(),
            unit: unit.to_string(),
            time_name: "Time".to_string(),
            time_unit: "s".to_string(),
            times: vec![0.0, 1.0],
            values: vec![val, val],
            valid: None,
        });
    }

    // Determine unified timebase
    let first_sig = signals.get(&req_channels[0]).unwrap();
    let time_name = first_sig.time_name.clone();
    let time_unit = first_sig.time_unit.clone();

    let all_same_times = req_channels.iter().all(|ch| {
        let s = signals.get(ch).unwrap();
        s.times.len() == first_sig.times.len() && s.times == first_sig.times
    });

    type ResampledSignals = HashMap<String, (Vec<f64>, Option<Vec<bool>>)>;

    let (times, resampled_signals): (Vec<f64>, ResampledSignals) = if all_same_times {
        let mut map = HashMap::new();
        for ch in &req_channels {
            let s = signals.get(ch).unwrap();
            map.insert(ch.clone(), (s.values.clone(), s.valid.clone()));
        }
        (first_sig.times.clone(), map)
    } else {
        // Build sorted unique union of timestamps
        let mut union_times = Vec::new();
        for ch in &req_channels {
            let s = signals.get(ch).unwrap();
            union_times.extend_from_slice(&s.times);
        }
        union_times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        union_times.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

        let mut map = HashMap::new();
        for ch in &req_channels {
            let s = signals.get(ch).unwrap();
            let (vals, valids) =
                resample_linear(&s.times, &s.values, s.valid.as_deref(), &union_times);
            map.insert(ch.clone(), (vals, valids));
        }
        (union_times, map)
    };

    let n_samples = times.len();
    if n_samples == 0 {
        return Err("all referenced channels have 0 samples".to_string());
    }

    let mut out_values = Vec::with_capacity(n_samples);
    let mut out_valid = Vec::with_capacity(n_samples);
    let mut any_invalid = false;

    for i in 0..n_samples {
        let (val, valid) = eval_point(expr, i, &resampled_signals)?;
        out_values.push(val);
        out_valid.push(valid);
        if !valid {
            any_invalid = true;
        }
    }

    Ok(ChannelSignal {
        loc: ChannelLoc {
            data_group_index: usize::MAX,
            channel_group_index: 0,
            channel_index: 0,
        },
        name: name.to_string(),
        unit: unit.to_string(),
        time_name,
        time_unit,
        times,
        values: out_values,
        valid: if any_invalid { Some(out_valid) } else { None },
    })
}

fn eval_scalar(expr: &Expr) -> Result<f64, String> {
    match expr {
        Expr::Number(n) => Ok(*n),
        Expr::Channel(name) => Err(format!("cannot evaluate channel '{name}' as a scalar")),
        Expr::Neg(inner) => Ok(-eval_scalar(inner)?),
        Expr::Add(a, b) => Ok(eval_scalar(a)? + eval_scalar(b)?),
        Expr::Sub(a, b) => Ok(eval_scalar(a)? - eval_scalar(b)?),
        Expr::Mul(a, b) => Ok(eval_scalar(a)? * eval_scalar(b)?),
        Expr::Div(a, b) => {
            let divisor = eval_scalar(b)?;
            if divisor.abs() < 1e-15 {
                Ok(f64::NAN)
            } else {
                Ok(eval_scalar(a)? / divisor)
            }
        }
    }
}

fn eval_point(
    expr: &Expr,
    idx: usize,
    signals: &HashMap<String, (Vec<f64>, Option<Vec<bool>>)>,
) -> Result<(f64, bool), String> {
    match expr {
        Expr::Number(n) => Ok((*n, true)),
        Expr::Channel(name) => {
            let Some((vals, valids)) = signals.get(name) else {
                return Err(format!("missing channel '{name}' during evaluation"));
            };
            let v = vals.get(idx).copied().unwrap_or(f64::NAN);
            let is_valid = match valids {
                Some(mask) => mask.get(idx).copied().unwrap_or(true),
                None => true,
            } && !v.is_nan();
            Ok((v, is_valid))
        }
        Expr::Neg(inner) => {
            let (v, valid) = eval_point(inner, idx, signals)?;
            Ok((-v, valid))
        }
        Expr::Add(a, b) => {
            let (va, valida) = eval_point(a, idx, signals)?;
            let (vb, validb) = eval_point(b, idx, signals)?;
            let is_valid = valida && validb;
            Ok((va + vb, is_valid))
        }
        Expr::Sub(a, b) => {
            let (va, valida) = eval_point(a, idx, signals)?;
            let (vb, validb) = eval_point(b, idx, signals)?;
            let is_valid = valida && validb;
            Ok((va - vb, is_valid))
        }
        Expr::Mul(a, b) => {
            let (va, valida) = eval_point(a, idx, signals)?;
            let (vb, validb) = eval_point(b, idx, signals)?;
            let is_valid = valida && validb;
            Ok((va * vb, is_valid))
        }
        Expr::Div(a, b) => {
            let (va, valida) = eval_point(a, idx, signals)?;
            let (vb, validb) = eval_point(b, idx, signals)?;
            if vb.abs() < 1e-15 {
                // Division by zero -> produce NaN and mark sample invalid
                Ok((f64::NAN, false))
            } else {
                let is_valid = valida && validb && !va.is_nan() && !vb.is_nan();
                Ok((va / vb, is_valid))
            }
        }
    }
}

/// Locates a channel in `file` by name (exact match first, then case-insensitive).
pub fn find_channel_loc(file: &Mf4File, name: &str) -> Option<ChannelLoc> {
    // Exact match
    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                if ch.name == name {
                    return Some(ChannelLoc {
                        data_group_index: dg_idx,
                        channel_group_index: cg_idx,
                        channel_index: ch_idx,
                    });
                }
            }
        }
    }
    // Case-insensitive fallback
    for (dg_idx, dg) in file.data_groups().iter().enumerate() {
        for (cg_idx, cg) in dg.channel_groups.iter().enumerate() {
            for (ch_idx, ch) in cg.channels.iter().enumerate() {
                if ch.name.eq_ignore_ascii_case(name) {
                    return Some(ChannelLoc {
                        data_group_index: dg_idx,
                        channel_group_index: cg_idx,
                        channel_index: ch_idx,
                    });
                }
            }
        }
    }
    None
}

/// Evaluates a [`ComputedDef`] against `file`, decoding referenced channels as needed.
pub fn evaluate_computed_channel(
    def: &ComputedDef,
    file: &Arc<Mf4File>,
    decoded_cache: &mut HashMap<ChannelLoc, ChannelSignal>,
) -> Result<ChannelSignal, String> {
    let expr = parse_expr(&def.expression)?;
    let req_names = expr.referenced_channels();

    let mut locs = Vec::new();
    for name in &req_names {
        let loc = find_channel_loc(file, name)
            .ok_or_else(|| format!("unknown channel '{name}' in expression"))?;
        locs.push((name.clone(), loc));

        if let std::collections::hash_map::Entry::Vacant(e) = decoded_cache.entry(loc) {
            match decode_channel(file, loc) {
                SignalLoadResult::Ok(sig) => {
                    e.insert(sig);
                }
                SignalLoadResult::Err { message } => {
                    return Err(format!("failed to decode channel '{name}': {message}"));
                }
            }
        }
    }

    let mut signals = HashMap::new();
    for (name, loc) in &locs {
        signals.insert(name.clone(), decoded_cache.get(loc).unwrap());
    }

    eval_expr(&def.name, &def.unit, &expr, &signals)
}

/// A cached computed-channel result, together with everything that can
/// invalidate it: the file the operands came from, and the shape of each
/// operand signal at evaluation time.
pub struct CachedComputed {
    file_id: usize,
    operands: Vec<(ChannelLoc, usize, usize)>,
    /// The evaluated signal, or the reason it could not be evaluated. Errors
    /// are cached too: re-parsing a broken expression every frame to print
    /// the same red line is the same waste as re-evaluating a good one.
    pub result: Result<Arc<ChannelSignal>, String>,
}

/// Evaluates the visible computed definitions, reusing cached results until a
/// definition, the file, or one of its operand signals changes.
///
/// `file_id` identifies the file the caches belong to; when it changes the
/// caller clears both caches, and anything cached under another id is
/// treated as stale. Hidden and empty definitions are skipped without any
/// work, so a definition that is not plotted costs nothing per frame.
/// Returns one `(index, result)` pair per evaluated definition, in
/// definition order.
pub fn evaluate_visible_defs(
    defs: &[ComputedDef],
    file: &Arc<Mf4File>,
    file_id: usize,
    operand_cache: &mut HashMap<ChannelLoc, ChannelSignal>,
    result_cache: &mut HashMap<ComputedDef, CachedComputed>,
) -> Vec<(usize, Result<Arc<ChannelSignal>, String>)> {
    // Definitions that were deleted or edited out of existence must not keep
    // their results alive; the cache is keyed by value, so a changed
    // definition simply stops matching and is re-evaluated below.
    result_cache.retain(|def, _| defs.iter().any(|current| current == def));

    let mut out = Vec::new();
    for (idx, def) in defs.iter().enumerate() {
        if !def.visible || (def.name.trim().is_empty() && def.expression.trim().is_empty()) {
            continue;
        }

        if let Some(cached) = result_cache.get(def) {
            let operands_unchanged = cached.operands.iter().all(|(loc, times, values)| {
                operand_cache
                    .get(loc)
                    .is_some_and(|sig| sig.times.len() == *times && sig.values.len() == *values)
            });
            if cached.file_id == file_id && operands_unchanged {
                out.push((idx, cached.result.clone()));
                continue;
            }
        }

        let result = evaluate_computed_channel(def, file, operand_cache).map(Arc::new);

        // Fingerprint the operands the successful evaluation was built from,
        // so a re-decoded channel invalidates the result. Failed evaluations
        // carry no operands and stay cached until the definition or file
        // changes.
        let operands = match &result {
            Ok(_) => parse_expr(&def.expression)
                .map(|expr| {
                    expr.referenced_channels()
                        .iter()
                        .filter_map(|name| {
                            let loc = find_channel_loc(file, name)?;
                            let sig = operand_cache.get(&loc)?;
                            Some((loc, sig.times.len(), sig.values.len()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };

        result_cache.insert(
            def.clone(),
            CachedComputed {
                file_id,
                operands,
                result: result.clone(),
            },
        );
        out.push((idx, result));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arithmetic_precedence() {
        let e = parse_expr("1 + 2 * 3").unwrap();
        assert_eq!(
            e,
            Expr::Add(
                Box::new(Expr::Number(1.0)),
                Box::new(Expr::Mul(
                    Box::new(Expr::Number(2.0)),
                    Box::new(Expr::Number(3.0))
                ))
            )
        );
    }

    #[test]
    fn parse_parentheses_override_precedence() {
        let e = parse_expr("(1 + 2) * 3").unwrap();
        assert_eq!(
            e,
            Expr::Mul(
                Box::new(Expr::Add(
                    Box::new(Expr::Number(1.0)),
                    Box::new(Expr::Number(2.0))
                )),
                Box::new(Expr::Number(3.0))
            )
        );
    }

    #[test]
    fn parse_left_associativity() {
        let e = parse_expr("10 - 4 - 2").unwrap();
        assert_eq!(
            e,
            Expr::Sub(
                Box::new(Expr::Sub(
                    Box::new(Expr::Number(10.0)),
                    Box::new(Expr::Number(4.0))
                )),
                Box::new(Expr::Number(2.0))
            )
        );
    }

    #[test]
    fn parse_unary_negation() {
        let e = parse_expr("-Speed * 2.0").unwrap();
        assert_eq!(
            e,
            Expr::Mul(
                Box::new(Expr::Neg(Box::new(Expr::Channel("Speed".to_string())))),
                Box::new(Expr::Number(2.0))
            )
        );
    }

    #[test]
    fn parse_bracketed_and_quoted_channel_names() {
        let e1 = parse_expr("[Wheel Speed FL] + [Wheel Speed FR]").unwrap();
        assert_eq!(
            e1,
            Expr::Add(
                Box::new(Expr::Channel("Wheel Speed FL".to_string())),
                Box::new(Expr::Channel("Wheel Speed FR".to_string()))
            )
        );

        let e2 = parse_expr(r#""Channel.Name" * 10.0"#).unwrap();
        assert_eq!(
            e2,
            Expr::Mul(
                Box::new(Expr::Channel("Channel.Name".to_string())),
                Box::new(Expr::Number(10.0))
            )
        );
    }

    #[test]
    fn parse_syntax_errors() {
        assert!(parse_expr("").is_err());
        assert!(parse_expr("1 +").is_err());
        assert!(parse_expr("(1 + 2").is_err());
        assert!(parse_expr("1 + 2)").is_err());
        assert!(parse_expr("1 @ 2").is_err());
        assert!(parse_expr("[unclosed").is_err());
    }

    #[test]
    fn eval_pointwise_same_timebase() {
        let sig_a = ChannelSignal {
            loc: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 0,
            },
            name: "A".to_string(),
            unit: "V".to_string(),
            time_name: "Time".to_string(),
            time_unit: "s".to_string(),
            times: vec![0.0, 1.0, 2.0],
            values: vec![10.0, 20.0, 30.0],
            valid: None,
        };
        let sig_b = ChannelSignal {
            loc: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 1,
            },
            name: "B".to_string(),
            unit: "A".to_string(),
            time_name: "Time".to_string(),
            time_unit: "s".to_string(),
            times: vec![0.0, 1.0, 2.0],
            values: vec![2.0, 4.0, 5.0],
            valid: None,
        };

        let mut signals = HashMap::new();
        signals.insert("A".to_string(), &sig_a);
        signals.insert("B".to_string(), &sig_b);

        let expr = parse_expr("A * B + 5.0").unwrap();
        let res = eval_expr("Power", "W", &expr, &signals).unwrap();

        assert_eq!(res.name, "Power");
        assert_eq!(res.unit, "W");
        assert_eq!(res.times, vec![0.0, 1.0, 2.0]);
        assert_eq!(res.values, vec![25.0, 85.0, 155.0]);
        assert_eq!(res.valid, None);
    }

    #[test]
    fn eval_different_timebases_resamples_onto_union() {
        let sig_10hz = ChannelSignal {
            loc: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 0,
            },
            name: "Slow".to_string(),
            unit: "m".to_string(),
            time_name: "Time".to_string(),
            time_unit: "s".to_string(),
            times: vec![0.0, 1.0, 2.0],
            values: vec![0.0, 10.0, 20.0],
            valid: None,
        };
        let sig_fast = ChannelSignal {
            loc: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 1,
                channel_index: 0,
            },
            name: "Fast".to_string(),
            unit: "m".to_string(),
            time_name: "Time".to_string(),
            time_unit: "s".to_string(),
            times: vec![0.0, 0.5, 1.0, 1.5, 2.0],
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0],
            valid: None,
        };

        let mut signals = HashMap::new();
        signals.insert("Slow".to_string(), &sig_10hz);
        signals.insert("Fast".to_string(), &sig_fast);

        let expr = parse_expr("Slow + Fast").unwrap();
        let res = eval_expr("Sum", "m", &expr, &signals).unwrap();

        assert_eq!(res.times, vec![0.0, 0.5, 1.0, 1.5, 2.0]);
        // Slow interpolated: at 0.0->0, 0.5->5, 1.0->10, 1.5->15, 2.0->20
        // Fast: 1, 2, 3, 4, 5
        assert_eq!(res.values, vec![1.0, 7.0, 13.0, 19.0, 25.0]);
    }

    #[test]
    fn eval_division_by_zero_produces_invalid_sample_without_panic() {
        let sig = ChannelSignal {
            loc: ChannelLoc {
                data_group_index: 0,
                channel_group_index: 0,
                channel_index: 0,
            },
            name: "X".to_string(),
            unit: "".to_string(),
            time_name: "Time".to_string(),
            time_unit: "s".to_string(),
            times: vec![0.0, 1.0, 2.0],
            values: vec![10.0, 20.0, 30.0],
            valid: None,
        };
        let mut signals = HashMap::new();
        signals.insert("X".to_string(), &sig);

        let expr = parse_expr("X / 0.0").unwrap();
        let res = eval_expr("DivZero", "", &expr, &signals).unwrap();

        assert_eq!(res.valid, Some(vec![false, false, false]));
        assert!(res.values.iter().all(|v| v.is_nan()));
    }

    fn nested_parens(levels: usize) -> String {
        format!("{}1{}", "(".repeat(levels), ")".repeat(levels))
    }

    #[test]
    fn nesting_at_the_depth_limit_still_parses() {
        // Exactly MAX_EXPRESSION_DEPTH levels of parens are legal; the bound
        // exists to catch pathological input, not to get in the way of a
        // plausible formula.
        let expr = parse_expr(&nested_parens(MAX_EXPRESSION_DEPTH)).unwrap();
        assert_eq!(expr, Expr::Number(1.0));
    }

    #[test]
    fn nesting_past_the_depth_limit_is_rejected_with_an_error() {
        // Without the bound this input recurses once per level until the
        // stack overflows, which aborts the process — not catchable. With the
        // bound it is an ordinary parse error the editor can display.
        let err = parse_expr(&nested_parens(MAX_EXPRESSION_DEPTH + 1)).unwrap_err();
        assert!(
            err.contains("depth"),
            "the error should say the nesting is too deep: {err}"
        );
    }

    #[test]
    fn a_wildly_nested_expression_is_rejected_before_it_can_hurt_anything() {
        // The shape a hostile paste or session line actually carries: far
        // beyond any limit. The parser must stop at the bound, not walk the
        // whole string recursively.
        let err = parse_expr(&nested_parens(10_000)).unwrap_err();
        assert!(err.contains("depth"));
    }

    #[test]
    fn deep_unary_chains_are_rejected_too() {
        // Unary operators recurse through parse_unary, so they are bounded by
        // the same counter as parentheses.
        let deep_neg = format!("{}1", "-".repeat(MAX_EXPRESSION_DEPTH + 1));
        assert!(parse_expr(&deep_neg).unwrap_err().contains("depth"));

        let ok_neg = format!("{}1", "-".repeat(MAX_EXPRESSION_DEPTH));
        assert!(parse_expr(&ok_neg).is_ok());
    }
}

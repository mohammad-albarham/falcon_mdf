//! Algebraic conversion formulas.
//!
//! MF4 conversion type 3 stores its rule as a text formula in a referenced TX
//! block, with `X` standing for the raw value — for example `4*X^2 + 1.5`.
//! This module parses such a formula once, into an expression tree that is then
//! evaluated per sample.
//!
//! The grammar is deliberately small: the arithmetic operators, parentheses,
//! and the mathematical functions the standard names. A formula using anything
//! outside that is rejected at parse time rather than silently mis-evaluated.

use crate::error::{Mf4Error, Result};

/// A parsed algebraic formula.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal number.
    Const(f64),
    /// The raw input value, written `X` (or `X1`) in the formula text.
    Variable,
    /// A unary negation.
    Neg(Box<Expr>),
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// A call to one of the supported mathematical functions.
    Call {
        /// Which function.
        func: Func,
        /// Its single argument.
        arg: Box<Expr>,
    },
}

/// A binary arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Division.
    Div,
    /// Exponentiation.
    Pow,
}

/// A supported single-argument mathematical function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    /// Sine.
    Sin,
    /// Cosine.
    Cos,
    /// Tangent.
    Tan,
    /// Arc sine.
    Asin,
    /// Arc cosine.
    Acos,
    /// Arc tangent.
    Atan,
    /// Hyperbolic sine.
    Sinh,
    /// Hyperbolic cosine.
    Cosh,
    /// Hyperbolic tangent.
    Tanh,
    /// Natural exponential.
    Exp,
    /// Natural logarithm.
    Log,
    /// Base-10 logarithm.
    Log10,
    /// Square root.
    Sqrt,
    /// Absolute value.
    Abs,
}

impl Func {
    fn from_name(name: &str) -> Option<Func> {
        Some(match name {
            "sin" => Func::Sin,
            "cos" => Func::Cos,
            "tan" => Func::Tan,
            "asin" => Func::Asin,
            "acos" => Func::Acos,
            "atan" => Func::Atan,
            "sinh" => Func::Sinh,
            "cosh" => Func::Cosh,
            "tanh" => Func::Tanh,
            "exp" => Func::Exp,
            "log" | "ln" => Func::Log,
            "log10" => Func::Log10,
            "sqrt" => Func::Sqrt,
            "abs" => Func::Abs,
            _ => return None,
        })
    }

    fn apply(self, x: f64) -> f64 {
        match self {
            Func::Sin => x.sin(),
            Func::Cos => x.cos(),
            Func::Tan => x.tan(),
            Func::Asin => x.asin(),
            Func::Acos => x.acos(),
            Func::Atan => x.atan(),
            Func::Sinh => x.sinh(),
            Func::Cosh => x.cosh(),
            Func::Tanh => x.tanh(),
            Func::Exp => x.exp(),
            Func::Log => x.ln(),
            Func::Log10 => x.log10(),
            Func::Sqrt => x.sqrt(),
            Func::Abs => x.abs(),
        }
    }
}

impl Expr {
    /// Parses a formula, with `X` denoting the raw value.
    pub fn parse(text: &str) -> Result<Expr> {
        let tokens = tokenize(text)?;
        let mut p = Parser { tokens, pos: 0 };
        let expr = p.expression()?;
        if p.pos != p.tokens.len() {
            return Err(bad(format!(
                "unexpected trailing input in formula '{text}'"
            )));
        }
        Ok(expr)
    }

    /// Evaluates the formula for one raw value.
    ///
    /// Arithmetic follows IEEE 754, so a domain error such as `sqrt(-1)` or a
    /// division by zero yields `NaN` or an infinity rather than failing.
    pub fn eval(&self, x: f64) -> f64 {
        match self {
            Expr::Const(v) => *v,
            Expr::Variable => x,
            Expr::Neg(inner) => -inner.eval(x),
            Expr::Call { func, arg } => func.apply(arg.eval(x)),
            Expr::Binary { op, lhs, rhs } => {
                let a = lhs.eval(x);
                let b = rhs.eval(x);
                match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => a / b,
                    BinOp::Pow => a.powf(b),
                }
            }
        }
    }
}

fn bad(msg: String) -> Mf4Error {
    Mf4Error::invalid_conversion(msg)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
}

fn tokenize(text: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' => {
                out.push(Token::Minus);
                i += 1;
            }
            '*' => {
                // Some writers spell exponentiation `**`.
                if chars.get(i + 1) == Some(&'*') {
                    out.push(Token::Caret);
                    i += 2;
                } else {
                    out.push(Token::Star);
                    i += 1;
                }
            }
            '/' => {
                out.push(Token::Slash);
                i += 1;
            }
            '^' => {
                out.push(Token::Caret);
                i += 1;
            }
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // Exponent notation, e.g. 1.5e-3
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let save = i;
                    i += 1;
                    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                        i += 1;
                    }
                    if i < chars.len() && chars[i].is_ascii_digit() {
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    } else {
                        i = save;
                    }
                }
                let s: String = chars[start..i].iter().collect();
                let v = s
                    .parse::<f64>()
                    .map_err(|_| bad(format!("invalid number '{s}' in formula")))?;
                out.push(Token::Num(v));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Token::Ident(chars[start..i].iter().collect()));
            }
            other => return Err(bad(format!("unexpected character '{other}' in formula"))),
        }
    }

    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, t: &Token) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// expression := term (('+' | '-') term)*
    fn expression(&mut self) -> Result<Expr> {
        let mut lhs = self.term()?;
        loop {
            let op = if self.eat(&Token::Plus) {
                BinOp::Add
            } else if self.eat(&Token::Minus) {
                BinOp::Sub
            } else {
                break;
            };
            let rhs = self.term()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    /// term := unary (('*' | '/') unary)*
    fn term(&mut self) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = if self.eat(&Token::Star) {
                BinOp::Mul
            } else if self.eat(&Token::Slash) {
                BinOp::Div
            } else {
                break;
            };
            let rhs = self.unary()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    /// unary := ('-' | '+')* power
    fn unary(&mut self) -> Result<Expr> {
        if self.eat(&Token::Minus) {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat(&Token::Plus) {
            return self.unary();
        }
        self.power()
    }

    /// power := primary ('^' unary)?  — right associative, and binding tighter
    /// than unary minus on the right so `2^-1` parses.
    fn power(&mut self) -> Result<Expr> {
        let base = self.primary()?;
        if self.eat(&Token::Caret) {
            let exp = self.unary()?;
            return Ok(Expr::Binary {
                op: BinOp::Pow,
                lhs: Box::new(base),
                rhs: Box::new(exp),
            });
        }
        Ok(base)
    }

    /// primary := number | variable | func '(' expression ')' | '(' expression ')'
    fn primary(&mut self) -> Result<Expr> {
        match self.peek().cloned() {
            Some(Token::Num(v)) => {
                self.pos += 1;
                Ok(Expr::Const(v))
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.expression()?;
                if !self.eat(&Token::RParen) {
                    return Err(bad("unbalanced parentheses in formula".into()));
                }
                Ok(inner)
            }
            Some(Token::Ident(name)) => {
                self.pos += 1;
                let lower = name.to_ascii_lowercase();

                // `X`, and the `X1`-style spelling used for single-input rules.
                if lower == "x" || lower == "x1" {
                    return Ok(Expr::Variable);
                }
                if lower == "pi" {
                    return Ok(Expr::Const(std::f64::consts::PI));
                }

                let Some(func) = Func::from_name(&lower) else {
                    return Err(bad(format!("unknown identifier '{name}' in formula")));
                };
                if !self.eat(&Token::LParen) {
                    return Err(bad(format!("expected '(' after function '{name}'")));
                }
                let arg = self.expression()?;
                if !self.eat(&Token::RParen) {
                    return Err(bad(format!("unbalanced parentheses after '{name}('")));
                }
                Ok(Expr::Call {
                    func,
                    arg: Box::new(arg),
                })
            }
            Some(other) => Err(bad(format!("unexpected token {other:?} in formula"))),
            None => Err(bad("formula ended unexpectedly".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(text: &str, x: f64) -> f64 {
        Expr::parse(text).expect("should parse").eval(x)
    }

    #[test]
    fn evaluates_the_identity_formula() {
        assert_eq!(eval("X", 3.5), 3.5);
    }

    #[test]
    fn respects_operator_precedence() {
        assert_eq!(eval("2 + 3 * 4", 0.0), 14.0);
        assert_eq!(eval("(2 + 3) * 4", 0.0), 20.0);
        assert_eq!(
            eval("10 - 2 - 3", 0.0),
            5.0,
            "subtraction is left associative"
        );
        assert_eq!(
            eval("100 / 5 / 2", 0.0),
            10.0,
            "division is left associative"
        );
    }

    #[test]
    fn exponentiation_is_right_associative_and_binds_tightest() {
        assert_eq!(eval("2^3^2", 0.0), 512.0, "2^(3^2), not (2^3)^2");
        assert_eq!(
            eval("2*X^2", 3.0),
            18.0,
            "power binds tighter than multiply"
        );
        assert_eq!(eval("-X^2", 3.0), -9.0, "negation applies after the power");
    }

    #[test]
    fn accepts_double_star_as_exponentiation() {
        assert_eq!(eval("X**2", 4.0), 16.0);
    }

    #[test]
    fn handles_negative_exponents() {
        assert_eq!(eval("2^-1", 0.0), 0.5);
    }

    #[test]
    fn parses_scientific_notation() {
        assert_eq!(eval("1.5e2", 0.0), 150.0);
        assert_eq!(eval("2e-3", 0.0), 0.002);
    }

    #[test]
    fn evaluates_a_realistic_conversion_formula() {
        // A typical sensor linearisation.
        assert!((eval("4*X^2 + 1.5*X - 0.25", 2.0) - 18.75).abs() < 1e-12);
    }

    #[test]
    fn supports_the_named_functions() {
        assert!((eval("sqrt(X)", 16.0) - 4.0).abs() < 1e-12);
        assert!((eval("abs(X)", -3.0) - 3.0).abs() < 1e-12);
        assert!((eval("exp(log(X))", 5.0) - 5.0).abs() < 1e-9);
        assert!((eval("sin(pi)", 0.0)).abs() < 1e-12);
    }

    #[test]
    fn is_case_insensitive_for_names() {
        assert_eq!(eval("SQRT(X)", 9.0), 3.0);
        assert_eq!(eval("x", 2.0), 2.0);
        assert_eq!(eval("X1", 2.0), 2.0);
    }

    #[test]
    fn rejects_rather_than_guesses_at_unknown_input() {
        assert!(Expr::parse("frobnicate(X)").is_err());
        assert!(Expr::parse("X +").is_err());
        assert!(Expr::parse("(X").is_err());
        assert!(Expr::parse("X)").is_err());
        assert!(Expr::parse("").is_err());
        assert!(Expr::parse("X $ 2").is_err());
    }

    #[test]
    fn ieee_semantics_for_domain_errors() {
        assert!(eval("sqrt(X)", -1.0).is_nan());
        assert!(eval("X/0", 1.0).is_infinite());
    }
}

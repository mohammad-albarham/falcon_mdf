//! Shared hand-rolled JSON parser for the wasm binding tests. No serde in
//! this crate's dependency tree, and the tests must parse exactly what the
//! binding emits.

#[derive(Debug, PartialEq)]
pub enum JsonVal {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonVal>),
    Obj(Vec<(String, JsonVal)>),
}

pub struct JsonParser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> JsonParser<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() {
                self.chars.next();
            } else {
                break;
            }
        }
    }

    pub fn parse_value(&mut self) -> Result<JsonVal, String> {
        self.skip_whitespace();
        let Some(&c) = self.chars.peek() else {
            return Err("Unexpected EOF".to_string());
        };

        match c {
            'n' => self.parse_null(),
            't' | 'f' => self.parse_bool(),
            '"' => self.parse_string().map(JsonVal::Str),
            '[' => self.parse_array(),
            '{' => self.parse_object(),
            '-' | '0'..='9' => self.parse_number(),
            other => Err(format!("Unexpected character: '{other}'")),
        }
    }

    fn parse_null(&mut self) -> Result<JsonVal, String> {
        for expected in "null".chars() {
            if self.chars.next() != Some(expected) {
                return Err("Expected 'null'".to_string());
            }
        }
        Ok(JsonVal::Null)
    }

    fn parse_bool(&mut self) -> Result<JsonVal, String> {
        if self.chars.peek() == Some(&'t') {
            for expected in "true".chars() {
                if self.chars.next() != Some(expected) {
                    return Err("Expected 'true'".to_string());
                }
            }
            Ok(JsonVal::Bool(true))
        } else {
            for expected in "false".chars() {
                if self.chars.next() != Some(expected) {
                    return Err("Expected 'false'".to_string());
                }
            }
            Ok(JsonVal::Bool(false))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.chars.next() != Some('"') {
            return Err("Expected '\"'".to_string());
        }

        let mut s = String::new();
        while let Some(c) = self.chars.next() {
            match c {
                '"' => return Ok(s),
                '\\' => match self.chars.next() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('b') => s.push('\x08'),
                    Some('f') => s.push('\x0C'),
                    Some('n') => s.push('\n'),
                    Some('r') => s.push('\r'),
                    Some('t') => s.push('\t'),
                    Some('u') => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            let h = self.chars.next().ok_or("Unterminated unicode escape")?;
                            hex.push(h);
                        }
                        let code = u32::from_str_radix(&hex, 16)
                            .map_err(|e| format!("Invalid hex escape: {e}"))?;
                        let ch = char::from_u32(code)
                            .ok_or_else(|| format!("Invalid char code: {code}"))?;
                        s.push(ch);
                    }
                    Some(other) => return Err(format!("Invalid escape sequence: \\{other}")),
                    None => return Err("Unterminated escape sequence".to_string()),
                },
                c if (c as u32) < 0x20 => {
                    return Err(format!("Unescaped control character: {:#x}", c as u32));
                }
                other => s.push(other),
            }
        }
        Err("Unterminated string".to_string())
    }

    fn parse_number(&mut self) -> Result<JsonVal, String> {
        let mut num_str = String::new();
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E' {
                num_str.push(c);
                self.chars.next();
            } else {
                break;
            }
        }
        let num: f64 = num_str
            .parse()
            .map_err(|e| format!("Invalid number '{num_str}': {e}"))?;
        Ok(JsonVal::Number(num))
    }

    fn parse_array(&mut self) -> Result<JsonVal, String> {
        if self.chars.next() != Some('[') {
            return Err("Expected '['".to_string());
        }

        let mut items = Vec::new();
        self.skip_whitespace();
        if self.chars.peek() == Some(&']') {
            self.chars.next();
            return Ok(JsonVal::Array(items));
        }

        loop {
            let val = self.parse_value()?;
            items.push(val);
            self.skip_whitespace();
            match self.chars.next() {
                Some(',') => self.skip_whitespace(),
                Some(']') => break,
                Some(other) => return Err(format!("Expected ',' or ']', found '{other}'")),
                None => return Err("Unterminated array".to_string()),
            }
        }
        Ok(JsonVal::Array(items))
    }

    fn parse_object(&mut self) -> Result<JsonVal, String> {
        if self.chars.next() != Some('{') {
            return Err("Expected '{'".to_string());
        }

        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.chars.peek() == Some(&'}') {
            self.chars.next();
            return Ok(JsonVal::Obj(entries));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.chars.next() != Some(':') {
                return Err("Expected ':' after key".to_string());
            }
            let val = self.parse_value()?;
            entries.push((key, val));
            self.skip_whitespace();
            match self.chars.next() {
                Some(',') => self.skip_whitespace(),
                Some('}') => break,
                Some(other) => return Err(format!("Expected ',' or '}}', found '{other}'")),
                None => return Err("Unterminated object".to_string()),
            }
        }
        Ok(JsonVal::Obj(entries))
    }
}

pub fn parse_json(input: &str) -> Result<JsonVal, String> {
    let mut parser = JsonParser::new(input);
    let val = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.chars.next().is_some() {
        return Err("Trailing characters after JSON value".to_string());
    }
    Ok(val)
}

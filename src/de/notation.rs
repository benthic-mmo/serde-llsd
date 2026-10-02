//! #  de/notation -- de-serialize LLSD, "notation" form.
//!
//!  Library for serializing and de-serializing data in
//!  Linden Lab Structured Data format.
//!
//!  Format documentation is at http://wiki.secondlife.com/wiki/LLSD
//!
//!  Notation format.
//!  Similar to JSON, but not compatible
//!
//! Notation format comes in two forms - bytes, and UTF-8 characters.
//! UTF-8 format is always valid UTF-8 strings, and can be encapsulated
//! inside XML if desired. This format is used inside SL/OS for "gltf material overrides".
//!
//! Byte string form is binary bytes, and cannot be encapsulated inside XML.
//! It can contain raw binary fields of the form b(NN)"rawbytes".
//! and raw strings of the form s(NN)"rawstring".
//! This form is used inside SL/OS for script uploads. We think.
//
//  Animats
//  June, 2023.
//  License: LGPL.

use crate::errors::ParseError;
use crate::{parse, LLSDValue};
use anyhow::anyhow;
use base64::Engine;
use chrono::DateTime;
use std::collections::HashMap;
use std::iter::Peekable;
use std::slice::Iter;
use std::str::{Bytes, Chars};
use uuid::Uuid;

/// Notation LLSD prefix
pub const LLSDNOTATIONPREFIX: &str = "<? llsd/notation ?>\n";

/// Sentinel, must match exactly.
pub const LLSDNOTATIONSENTINEL: &str = LLSDNOTATIONPREFIX;

/// Exported parse from bytes.
pub fn from_bytes(b: &[u8]) -> Result<LLSDValue, ParseError> {
    LLSDStreamBytes::parse(b)
}

/// Exported parse from str.
pub fn from_str(s: &str) -> Result<LLSDValue, ParseError> {
    LLSDStreamChars::parse(s)
}

/// An LLSD stream. May be either a UTF-8 stream or a byte stream.
trait LLSDStream<C, S> {
    /// Get next char/byte.
    fn next(&mut self) -> Option<C>;

    /// Get next char/byte, result.
    fn next_ok(&mut self) -> Result<C, ParseError> {
        self.next()
            .ok_or_else(|| ParseError::new(anyhow!("Unexpected end of input parsing Notation")))
    }

    /// Peek at next char/byte.
    fn peek(&mut self) -> Option<&C>;

    /// Peek at next char, as result.
    fn peek_ok(&mut self) -> Result<&C, ParseError> {
        self.peek()
            .ok_or_else(|| ParseError::new(anyhow!("Unexpected end of input parsing Notation")))
    }

    /// Convert into char.
    fn into_char(ch: &C) -> char;

    /// Consume whitespace. Next char will be non-whitespace.
    fn consume_whitespace(&mut self) -> Result<(), ParseError> {
        while let Some(ch) = self.peek() {
            match Self::into_char(ch) {
                ' ' | '\n' => {
                    let _ = self.next();
                }
                '\\' => {
                    let _ = self.next();

                    let ch = Self::into_char(&self.next_ok()?);

                    if ch != 'n' {
                        return Err(ParseError::new(anyhow!(
                            "Unexpected escape sequence \"\\{}\" where white space expected.",
                            ch
                        )));
                    }
                }
                _ => break,
            }
        }

        Ok(())
    }

    /// Consume expected non-whitespace char.
    fn consume_char(&mut self, expected_ch: char) -> Result<(), ParseError> {
        self.consume_whitespace()?;

        let ch = Self::into_char(&self.next_ok()?);

        if ch == expected_ch {
            Ok(())
        } else {
            Err(ParseError::new(anyhow!(
                "Expected '{}', found '{}'.",
                expected_ch,
                ch
            )))
        }
    }

    /// Parse "iNNN".
    fn parse_integer(&mut self) -> Result<LLSDValue, ParseError> {
        let mut s = String::with_capacity(20);

        while let Some(ch) = self.peek() {
            match Self::into_char(ch) {
                '0'..='9' | '+' | '-' => {
                    s.push(Self::into_char(&self.next().unwrap()));
                }
                _ => break,
            }
        }

        let value = parse!(s.parse::<i32>())?;

        Ok(LLSDValue::Integer(value))
    }

    /// Parse "rNNN".
    fn parse_real(&mut self) -> Result<LLSDValue, ParseError> {
        let mut s = String::with_capacity(20);

        while let Some(ch) = self.peek() {
            match Self::into_char(ch) {
                '0'..='9' | '+' | '-' | '.' => {
                    s.push(Self::into_char(&self.next().unwrap()));
                }
                _ => break,
            }
        }

        let value = parse!(s.parse::<f64>())?;

        Ok(LLSDValue::Real(value))
    }

    /// Parse Boolean.
    fn parse_boolean(&mut self, first_char: char) -> Result<LLSDValue, ParseError> {
        let mut s = String::with_capacity(4);
        s.push(first_char);

        loop {
            if let Some(ch) = self.peek() {
                if Self::into_char(ch).is_alphabetic() {
                    s.push(Self::into_char(&self.next().unwrap()));
                    continue;
                }
            }

            break;
        }

        match s.as_str() {
            "f" | "F" | "false" | "FALSE" => Ok(LLSDValue::Boolean(false)),
            "t" | "T" | "true" | "TRUE" => Ok(LLSDValue::Boolean(true)),
            _ => Err(ParseError::new(anyhow!("Parsing Boolean, got {}", s))),
        }
    }

    /// Parse string. "ABC" or 'ABC', with '\' as escape.
    /// Allowed escapes are \\, \", \', and \n.
    /// Does not parse the numeric count prefix form.
    fn parse_quoted_string(&mut self, delim: char) -> Result<String, ParseError> {
        self.consume_whitespace()?;

        let mut s = String::with_capacity(128);

        loop {
            let ch = Self::into_char(&self.next_ok()?);

            if ch == delim {
                break;
            }

            if ch == '\\' {
                let ch = Self::into_char(&self.next_ok()?);

                match ch {
                    '\\' | '\'' | '"' => s.push(ch),
                    'n' => s.push('\n'),
                    _ => {
                        return Err(ParseError::new(anyhow!(
                            "Unexpected escape sequence \"\\{}\" within quoted string.",
                            ch
                        )));
                    }
                }
            } else {
                s.push(ch);
            }
        }

        s.shrink_to_fit();

        Ok(s)
    }

    /// Parse date string per RFC 3339.
    fn parse_date(&mut self) -> Result<LLSDValue, ParseError> {
        if let Some(delim) = self.next() {
            let delim = Self::into_char(&delim);

            if delim == '"' || delim == '\'' {
                let s = self.parse_quoted_string(delim)?;
                let date = parse!(DateTime::parse_from_rfc3339(&s))?;

                Ok(LLSDValue::Date(date.timestamp()))
            } else {
                Err(ParseError::new(anyhow!("Date did not begin with '\"'")))
            }
        } else {
            Err(ParseError::new(anyhow!("Date at end of file.")))
        }
    }

    /// Parse URI string per RFC 1738.
    fn parse_uri(&mut self) -> Result<LLSDValue, ParseError> {
        if let Some(delim) = self.next() {
            let delim = Self::into_char(&delim);

            if delim == '"' || delim == '\'' {
                let s = self.parse_quoted_string(delim)?;
                let uri = parse!(urlencoding::decode(&s))?;

                Ok(LLSDValue::URI(uri.to_string()))
            } else {
                Err(ParseError::new(anyhow!("URI did not begin with '\"'")))
            }
        } else {
            Err(ParseError::new(anyhow!("URI at end of file.")))
        }
    }

    /// Parse UUID. No quotes.
    fn parse_uuid(&mut self) -> Result<LLSDValue, ParseError> {
        const UUID_LEN: usize = "c69b29b1-8944-58ae-a7c5-2ca7b23e22fb".len();

        let mut s = String::with_capacity(UUID_LEN);

        for _ in 0..UUID_LEN {
            let ch = self
                .next()
                .ok_or_else(|| ParseError::new(anyhow!("EOF parsing UUID")))?;

            s.push(Self::into_char(&ch));
        }

        let uuid = parse!(Uuid::parse_str(&s))?;

        Ok(LLSDValue::UUID(uuid))
    }

    /// Parse "{ 'key' : value, 'key' : value ... }".
    fn parse_map(&mut self) -> Result<LLSDValue, ParseError> {
        let mut kvmap = HashMap::new();

        loop {
            self.consume_whitespace()?;

            let key = {
                let ch = Self::into_char(&self.next_ok()?);

                match ch {
                    '}' => break,
                    '\'' | '"' => self.parse_quoted_string(ch)?,
                    _ => {
                        return Err(ParseError::new(anyhow!(
                            "Map key began with {} instead of quote.",
                            ch
                        )));
                    }
                }
            };

            self.consume_char(':')?;

            let value = self.parse_value()?;
            kvmap.insert(key, value);

            self.consume_whitespace()?;

            if Self::into_char(self.peek_ok()?) == ',' {
                let _ = self.next();
            }
        }

        Ok(LLSDValue::Map(kvmap))
    }

    /// Parse "[ value, value ... ]".
    fn parse_array(&mut self) -> Result<LLSDValue, ParseError> {
        let mut array_items = Vec::new();

        loop {
            self.consume_whitespace()?;

            let ch = Self::into_char(self.peek_ok()?);

            if ch == ']' {
                let _ = self.next();
                break;
            }

            array_items.push(self.parse_value()?);

            self.consume_whitespace()?;

            if Self::into_char(self.peek_ok()?) == ',' {
                let _ = self.next();
            }
        }

        Ok(LLSDValue::Array(array_items))
    }

    fn parse_binary(&mut self) -> Result<LLSDValue, ParseError>;

    fn parse_sized_string(&mut self) -> Result<LLSDValue, ParseError>;

    /// Parse one value - real, integer, map, etc. Recursive.
    fn parse_value(&mut self) -> Result<LLSDValue, ParseError> {
        self.consume_whitespace()?;

        let ch = Self::into_char(&self.next_ok()?);

        match ch {
            '!' => Ok(LLSDValue::Undefined),
            '0' => Ok(LLSDValue::Boolean(false)),
            '1' => Ok(LLSDValue::Boolean(true)),
            'f' | 'F' => self.parse_boolean(ch),
            't' | 'T' => self.parse_boolean(ch),
            '{' => self.parse_map(),
            '[' => self.parse_array(),
            'i' => self.parse_integer(),
            'r' => self.parse_real(),
            'd' => self.parse_date(),
            'u' => self.parse_uuid(),
            'l' => self.parse_uri(),
            'b' => self.parse_binary(),
            's' => self.parse_sized_string(),
            '"' | '\'' => Ok(LLSDValue::String(self.parse_quoted_string(ch)?)),
            _ => Err(ParseError::new(anyhow!("Unexpected character: {:?}", ch))),
        }
    }
}

/// Stream, composed of UTF-8 chars.
struct LLSDStreamChars<'a> {
    cursor: Peekable<Chars<'a>>,
}

impl LLSDStream<char, Peekable<Chars<'_>>> for LLSDStreamChars<'_> {
    fn next(&mut self) -> Option<char> {
        self.cursor.next()
    }

    fn peek(&mut self) -> Option<&char> {
        self.cursor.peek()
    }

    fn into_char(ch: &char) -> char {
        *ch
    }

    fn parse_binary(&mut self) -> Result<LLSDValue, ParseError> {
        Err(ParseError::new(anyhow!(
            "Byte-counted binary data inside UTF-8 won't work."
        )))
    }

    fn parse_sized_string(&mut self) -> Result<LLSDValue, ParseError> {
        Err(ParseError::new(anyhow!(
            "Byte-counted string data inside UTF-8 won't work."
        )))
    }
}

impl LLSDStreamChars<'_> {
    /// Parse LLSD notation from a string.
    /// No header.
    pub fn parse(notation_str: &str) -> Result<LLSDValue, ParseError> {
        let mut stream = LLSDStreamChars {
            cursor: notation_str.chars().peekable(),
        };

        match stream.parse_value() {
            Ok(value) => Ok(value),
            Err(e) => {
                let s = beginning_to_iterator(notation_str, &stream.cursor);

                Err(ParseError::new(anyhow!(
                    "LLSD notation string parse error: {}. Parse got this far: {}",
                    e,
                    s
                )))
            }
        }
    }
}

/// Stream, composed of raw bytes.
struct LLSDStreamBytes<'a> {
    cursor: Peekable<Iter<'a, u8>>,
}

impl LLSDStream<u8, Peekable<Bytes<'_>>> for LLSDStreamBytes<'_> {
    fn next(&mut self) -> Option<u8> {
        self.cursor.next().copied()
    }

    fn peek(&mut self) -> Option<&u8> {
        self.cursor.peek().copied()
    }

    fn into_char(ch: &u8) -> char {
        (*ch).into()
    }

    /// Parse binary value.
    ///
    /// Format is b16"value", b64"value", or b(cnt)"value".
    fn parse_binary(&mut self) -> Result<LLSDValue, ParseError> {
        if let Some(ch) = self.peek() {
            match Self::into_char(ch) {
                '(' => {
                    let cnt = self.parse_number_in_parentheses()?;

                    self.consume_char('"')?;

                    let s = self.next_chunk(cnt)?;

                    self.consume_char('"')?;

                    Ok(LLSDValue::Binary(s))
                }

                '1' => {
                    self.consume_char('1')?;
                    self.consume_char('6')?;
                    self.consume_char('"')?;

                    let mut s = self.parse_quoted_string('"')?;
                    s.retain(|c| !c.is_whitespace());

                    let bytes = parse!(hex::decode(s))?;

                    Ok(LLSDValue::Binary(bytes))
                }

                '6' => {
                    self.consume_char('6')?;
                    self.consume_char('4')?;
                    self.consume_char('"')?;

                    let mut s = self.parse_quoted_string('"')?;
                    s.retain(|c| !c.is_whitespace());

                    let bytes = parse!(base64::engine::general_purpose::STANDARD.decode(s))?;

                    Ok(LLSDValue::Binary(bytes))
                }

                _ => Err(ParseError::new(anyhow!(
                    "Binary value started with {} instead of (, 1, or 6",
                    ch
                ))),
            }
        } else {
            Err(ParseError::new(anyhow!("Binary value started with EOF")))
        }
    }

    /// Parse sized string.
    ///
    /// Format is s(NNN)"string".
    fn parse_sized_string(&mut self) -> Result<LLSDValue, ParseError> {
        let cnt = self.parse_number_in_parentheses()?;

        self.consume_char('"')?;

        let s = self.next_chunk(cnt)?;

        self.consume_char('"')?;

        let string = parse!(String::from_utf8(s))?;

        Ok(LLSDValue::String(string))
    }
}

impl LLSDStreamBytes<'_> {
    /// Parse `(NNN)`, which is used for length information.
    fn parse_number_in_parentheses(&mut self) -> Result<usize, ParseError> {
        self.consume_char('(')?;

        let val = self.parse_integer()?;

        self.consume_char(')')?;

        match val {
            LLSDValue::Integer(v) => usize::try_from(v)
                .map_err(|_| ParseError::new(anyhow!("Negative value used as byte count: {}", v))),
            _ => Err(ParseError::new(anyhow!(
                "Integer parse did not return an integer."
            ))),
        }
    }

    /// Read chunk of N bytes.
    fn next_chunk(&mut self, cnt: usize) -> Result<Vec<u8>, ParseError> {
        let mut s = Vec::with_capacity(cnt);

        for _ in 0..cnt {
            s.push(self.next_ok()?);
        }

        Ok(s)
    }

    /// Parse LLSD string expressed in notation format into an LLSDObject tree.
    /// No header.
    pub fn parse(notation_bytes: &[u8]) -> Result<LLSDValue, ParseError> {
        let mut stream = LLSDStreamBytes {
            cursor: notation_bytes.iter().peekable(),
        };

        stream.parse_value()
    }
}

/// Extract the part of a string from the beginning to an iterator.
fn beginning_to_iterator<'a>(orig: &'a str, pos: &Peekable<Chars<'_>>) -> &'a str {
    let suffix: String = pos.clone().collect();

    if let Some(s) = orig.strip_suffix(&suffix) {
        s
    } else {
        orig
    }
}

#[test]
fn notationparse1() {
    let s1 = "\"ABC☺DEF\"".to_string();

    let mut stream1 = LLSDStreamChars {
        cursor: s1.chars().peekable(),
    };

    stream1.consume_char('"').unwrap();

    let v1 = stream1.parse_quoted_string('"').unwrap();

    assert_eq!(v1, "ABC☺DEF");
}

#[test]
fn notationparse2() {
    const TESTNOTATION2: &str = r#"
[
  {'destination':l"http://secondlife.com"}, 
  {'version':i1}, 
  {
    'agent_id':u3c115e51-04f4-523c-9fa6-98aff1034730, 
    'session_id':u2c585cec-038c-40b0-b42e-a25ebab4d132, 
    'circuit_code':i1075, 
    'first_name':'Phoenix', 
    'last_name':'Linden',
    'position':[r70.9247,r254.378,r38.7304], 
    'look_at':[r-0.043753,r-0.999042,r0], 
    'granters':[ua2e76fcd-9360-4f6d-a924-000000000003],
    'attachment_data':
    [
      {
        'attachment_point':i2,
        'item_id':ud6852c11-a74e-309a-0462-50533f1ef9b3,
        'asset_id':uc69b29b1-8944-58ae-a7c5-2ca7b23e22fb
      },
      {
        'attachment_point':i10, 
        'item_id':uff852c22-a74e-309a-0462-50533f1ef900,
        'asset_id':u5868dd20-c25a-47bd-8b4c-dedc99ef9479
      }
    ]
  }
]
"#;

    let parsed_s = LLSDStreamChars::parse(TESTNOTATION2);

    println!("Parse of string form {}: \n{:#?}", TESTNOTATION2, parsed_s);

    let parsed_b = LLSDStreamBytes::parse(TESTNOTATION2.as_bytes());

    println!("Parse of byte form: {:#?}", parsed_b);

    assert_eq!(parsed_s.unwrap(), parsed_b.unwrap());
}

#[test]
fn notationparse3() {
    const TESTNOTATION3: &str = r#"
[
  {
    'creation-date':d"2007-03-15T18:30:18Z", 
    'creator-id':u3c115e51-04f4-523c-9fa6-98aff1034730
  },
  s(10)"0123456789",
  "Where's the beef?",
  'Over here.',  
  b(158)"default
{
    state_entry()
    {
        llSay(0, "Hello, Avatar!");
    }

    touch_start(integer total_number)
    {
        llSay(0, "Touched.");
    }
}",
  b64"AABAAAAAAAAAAAIAAAA//wAAP/8AAADgAAAA5wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
AABkAAAAZAAAAAAAAAAAAAAAZAAAAAAAAAABAAAAAAAAAAAAAAAAAAAABQAAAAEAAAAQAAAAAAAA
AAUAAAAFAAAAABAAAAAAAAAAPgAAAAQAAAAFAGNbXgAAAABgSGVsbG8sIEF2YXRhciEAZgAAAABc
XgAAAAhwEQjRABeVAAAABQBjW14AAAAAYFRvdWNoZWQuAGYAAAAAXF4AAAAIcBEI0QAXAZUAAEAA
AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" 
]
"#;

    let parsed_b = from_bytes(TESTNOTATION3.as_bytes()).unwrap();

    println!("Parse of byte form: {:#?}", parsed_b);

    let parsed_b = from_str(TESTNOTATION3);

    assert!(parsed_b.is_err());

    println!("Parse of string form: {:#?}", parsed_b);
}

#[test]
fn notationparse4() {
    const TESTNOTATION4: &str = r#"
        {'gltf_json':['{\"asset\":{\"version\":\"2.0\"},\"images\":[{\"uri\":\"5748decc-f629-461c-9a36-a35a221fe21f\"},
            {\"uri\":\"5748decc-f629-461c-9a36-a35a221fe21f\"}],\"materials\":[{\"occlusionTexture\":{\"index\":1},\"pbrMetallicRoughness\":{\"metallicRoughnessTexture\":{\"index\":0},\"roughnessFactor\":0.20000000298023224}}],\"textures\":[{\"source\":0},
            {\"source\":1}]}\\n'],'local_id':i8893800,'object_id':u6ac43d70-80eb-e526-ec91-110b4116293e,'region_handle_x':i342016,'region_handle_y':i343552,'sides':[i0]}"
"#;

    let parsed_b = from_bytes(TESTNOTATION4.as_bytes());

    println!("Parse of byte form: {:#?}", parsed_b);

    let local_id = *parsed_b
        .unwrap()
        .as_map()
        .unwrap()
        .get("local_id")
        .unwrap()
        .as_integer()
        .unwrap();

    assert_eq!(local_id, 8893800);

    let parsed_b = from_str(TESTNOTATION4);

    println!("Parse of str form: {:#?}", parsed_b);

    let local_id = *parsed_b
        .unwrap()
        .as_map()
        .unwrap()
        .get("local_id")
        .unwrap()
        .as_integer()
        .unwrap();

    assert_eq!(local_id, 8893800);
}

#[test]
fn notationparse5() {
    const TESTNOTATION5: &str = r#"
        {'gltf_json':['{\"asset\":{\"version\":\"2.0\"},\"im\ages\":[{\"uri\":\"5748decc-f629-461c-9a36-a35a221fe21f\"},
            {\"uri\":\"5748decc-f629-461c-9a36-a35a221fe21f\"}],\"materials\":[{\"occlusionTexture\":{\"index\":1},\"pbrMetallicRoughness\":{\"metallicRoughnessTexture\":{\"index\":0},\"roughnessFactor\":0.20000000298023224}}],\"textures\":[{\"source\":0},
            {\"source\":1}]}\\n'],'local_id':i8893800,'object_id':u6ac43d70-80eb-e526-ec91-110b4116293e,'region_handle_x':i342016,'region_handle_y':i343552,'sides':[i0]}"
"#;

    println!("Test notationparse5");

    let parsed_b = from_str(TESTNOTATION5);

    println!("Parse of byte form: {:#?}", parsed_b);

    assert!(parsed_b.is_err());
}

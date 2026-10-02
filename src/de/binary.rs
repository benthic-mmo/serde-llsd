//! #  de/binary -- de-serialize LLSD, binary form.
//!
//!  Library for serializing and de-serializing data in
//!  Linden Lab Structured Data format.
//!
//!  Format documentation is at http://wiki.secondlife.com/wiki/LLSD
//!
//!  Binary format.
//
//  Animats
//  March, 2021.
//  License: LGPL.
//
use crate::errors::ParseError;
use crate::{parse, LLSDValue};
use anyhow::{anyhow, Error};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use uuid;
//
//  Constants
//
pub const LLSDBINARYPREFIX: &[u8] = b"<? LLSD/Binary ?>\n"; // binary LLSD prefix
pub const LLSDBINARYSENTINEL: &[u8] = LLSDBINARYPREFIX; // prefix must match exactly

///    Parse LLSD array expressed in binary into an LLSDObject tree. No header.
pub fn from_bytes(b: &[u8]) -> Result<LLSDValue, ParseError> {
    let mut cursor: Cursor<&[u8]> = Cursor::new(b);
    parse_value(&mut cursor)
}

///    Parse LLSD reader expressed in binary into an LLSDObject tree. No header.
pub fn from_reader(cursor: &mut dyn Read) -> Result<LLSDValue, ParseError> {
    parse_value(cursor)
}

/// Parse one value - real, integer, map, etc. Recursive.
fn parse_value(cursor: &mut dyn Read) -> Result<LLSDValue, ParseError> {
    // These could be generic if generics with numeric parameters were in stable Rust.
    fn read_u8(cursor: &mut dyn Read) -> Result<u8, Error> {
        let mut b: [u8; 1] = [0; 1];
        cursor.read_exact(&mut b)?;
        Ok(b[0])
    }

    fn read_u32(cursor: &mut dyn Read) -> Result<u32, Error> {
        let mut b: [u8; 4] = [0; 4];
        cursor.read_exact(&mut b)?;
        Ok(u32::from_be_bytes(b))
    }

    fn read_i32(cursor: &mut dyn Read) -> Result<i32, Error> {
        let mut b: [u8; 4] = [0; 4];
        cursor.read_exact(&mut b)?;
        Ok(i32::from_be_bytes(b))
    }

    fn read_i64(cursor: &mut dyn Read) -> Result<i64, Error> {
        let mut b: [u8; 8] = [0; 8];
        cursor.read_exact(&mut b)?;
        Ok(i64::from_be_bytes(b))
    }

    fn read_f64(cursor: &mut dyn Read) -> Result<f64, Error> {
        let mut b: [u8; 8] = [0; 8];
        cursor.read_exact(&mut b)?;
        Ok(f64::from_be_bytes(b))
    }

    fn read_variable(cursor: &mut dyn Read) -> Result<Vec<u8>, Error> {
        let length = read_u32(cursor)?;
        let mut buf = vec![0u8; length as usize];
        cursor.read_exact(&mut buf)?;
        Ok(buf)
    }

    let typecode = parse!(read_u8(cursor))?;

    match typecode {
        b'!' => Ok(LLSDValue::Undefined),

        b'0' => Ok(LLSDValue::Boolean(false)),
        b'1' => Ok(LLSDValue::Boolean(true)),

        b's' => Ok(LLSDValue::String(
            parse!(std::str::from_utf8(&parse!(read_variable(cursor))?))?.to_string(),
        )),

        b'l' => Ok(LLSDValue::URI(
            parse!(std::str::from_utf8(&parse!(read_variable(cursor))?))?.to_string(),
        )),

        b'i' => Ok(LLSDValue::Integer(parse!(read_i32(cursor))?)),

        b'r' => Ok(LLSDValue::Real(parse!(read_f64(cursor))?)),

        b'u' => {
            let mut buf: [u8; 16] = [0u8; 16];
            parse!(cursor.read_exact(&mut buf))?;

            Ok(LLSDValue::UUID(uuid::Uuid::from_bytes(buf)))
        }

        b'b' => Ok(LLSDValue::Binary(parse!(read_variable(cursor))?)),

        b'd' => Ok(LLSDValue::Date(parse!(read_i64(cursor))?)),

        b'{' => {
            let mut dict: HashMap<String, LLSDValue> = HashMap::new();
            let count = parse!(read_u32(cursor))?;

            for _ in 0..count {
                let keyprefix = parse!(read_u8(cursor))?;

                match keyprefix {
                    b'k' => {
                        let key = parse!(std::str::from_utf8(&parse!(read_variable(cursor))?))?
                            .to_string();

                        let _ = dict.insert(key, parse_value(cursor)?);
                    }

                    _ => {
                        return Err(ParseError::new(anyhow!(
                            "Binary LLSD map key had {:?} instead of expected 'k'",
                            keyprefix
                        )));
                    }
                }
            }

            if parse!(read_u8(cursor))? != b'}' {
                return Err(ParseError::new(anyhow!(
                    "Binary LLSD map did not end properly with }}"
                )));
            }

            Ok(LLSDValue::Map(dict))
        }

        b'[' => {
            let mut array: Vec<LLSDValue> = Vec::new();
            let count = parse!(read_u32(cursor))?;

            for _ in 0..count {
                array.push(parse_value(cursor)?);
            }

            if parse!(read_u8(cursor))? != b']' {
                return Err(ParseError::new(anyhow!(
                    "Binary LLSD array did not end properly with ]"
                )));
            }

            Ok(LLSDValue::Array(array))
        }

        _ => Err(ParseError::new(anyhow!(
            "Binary LLSD, unexpected type code {:?}",
            typecode
        ))),
    }
}
// Unit test

#[test]
fn binaryparsetest1() {
    //  Construct a test value.
    let test1map: HashMap<String, LLSDValue> = [
        ("val1".to_string(), LLSDValue::Real(456.0)),
        ("val2".to_string(), LLSDValue::Integer(999)),
    ]
    .iter()
    .cloned()
    .collect();
    let test1: LLSDValue = LLSDValue::Array(vec![
        LLSDValue::Real(123.5),
        LLSDValue::Map(test1map),
        LLSDValue::Integer(42),
        LLSDValue::String("Hello world".to_string()),
    ]);
    //  Convert to binary form.
    let test1bin = crate::to_bytes(&test1).unwrap();
    println!("Binary form: {:?}", test1bin);
    //  Convert back to value form.
    let test1value = from_bytes(&test1bin[LLSDBINARYSENTINEL.len()..]).unwrap();
    println!("Value after round-trip conversion: {:?}", test1value);
    //  Check that results match after round trip.
    assert_eq!(test1, test1value);
}

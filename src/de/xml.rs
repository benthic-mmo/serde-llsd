//
//  de/xml.rs -- XML deserializer for LLSD
//
//  Library for serializing and de-serializing data in
//  Linden Lab Structured Data format.
//
//  Format documentation is at http://wiki.secondlife.com/wiki/LLSD
//
//  XML format.
//
//  Animats
//  February, 2021.
//  License: LGPL.
//
use crate::errors::ParseError;
use crate::{parse, LLSDValue};
use anyhow::anyhow;
use ascii85;
use base64;
use base64::Engine;
use quick_xml::events::attributes::Attributes;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};

//
//  Constants
//
pub const LLSDXMLPREFIX: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<llsd>\n";
pub const LLSDXMLSENTINEL: &str = "<?xml"; // Must begin with this.

/// Parse LLSD expressed in XML into an LLSD tree.
pub fn from_str(xmlstr: &str) -> Result<LLSDValue, ParseError> {
    let result = from_reader(&mut BufReader::new(xmlstr.as_bytes()));

    if let Err(e) = &result {
        return Err(ParseError::new(anyhow!("{}: {}", e, xmlstr)));
    }

    result
}

/// Read XML from buffered source and parse into LLSDValue.
fn from_reader<R: BufRead>(rdr: &mut R) -> Result<LLSDValue, ParseError> {
    let mut reader = Reader::from_reader(rdr);
    reader.trim_text(true);
    reader.expand_empty_elements(true);

    let mut buf = Vec::new();
    let mut output: Option<LLSDValue> = None;

    // Outer parse. Find <llsd> and parse its interior.
    loop {
        match reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name() {
                b"llsd" => {
                    if output.is_some() {
                        return Err(ParseError::new(anyhow!(
                            "More than one <llsd> block in data"
                        )));
                    }

                    let mut buf2 = Vec::new();

                    match reader.read_event(&mut buf2) {
                        Ok(Event::Start(ref e)) => {
                            let tagname = parse!(std::str::from_utf8(e.name()))?;

                            output = Some(parse_value(&mut reader, tagname, &e.attributes())?);
                        }

                        _ => {
                            return Err(ParseError::new(anyhow!(
                                "Expected LLSD data, found {:?} error at position {}",
                                e.name(),
                                reader.buffer_position()
                            )));
                        }
                    }
                }

                _ => {
                    return Err(ParseError::new(anyhow!(
                        "Expected <llsd>, found {:?} error at position {}",
                        e.name(),
                        reader.buffer_position()
                    )));
                }
            },

            Ok(Event::Text(_e)) => {}

            Ok(Event::End(ref _e)) => {}

            Ok(Event::Eof) => break,

            Err(e) => {
                return Err(ParseError::new(anyhow!(
                    "Error at position {}: {:?}",
                    reader.buffer_position(),
                    e
                )));
            }

            _ => {}
        }

        buf.clear();
    }

    match output {
        Some(out) => Ok(out),

        None => Err(ParseError::new(anyhow!(
            "Unexpected end of data, no <llsd> block."
        ))),
    }
}

/// Parse one value - real, integer, map, etc. Recursive.
fn parse_value<R: BufRead>(
    reader: &mut Reader<&mut R>,
    starttag: &str,
    attrs: &Attributes,
) -> Result<LLSDValue, ParseError> {
    match starttag {
        "undef" | "real" | "integer" | "boolean" | "string" | "uri" | "binary" | "uuid"
        | "date" => parse_primitive_value(reader, starttag, attrs),

        "map" => parse_map(reader),

        "array" => parse_array(reader, parse_value),

        _ => Err(ParseError::new(anyhow!(
            "Unknown data type <{}> at position {}",
            starttag,
            reader.buffer_position()
        ))),
    }
}

/// Parse one primitive value.
fn parse_primitive_value<R: BufRead>(
    reader: &mut Reader<&mut R>,
    starttag: &str,
    attrs: &Attributes,
) -> Result<LLSDValue, ParseError> {
    let mut texts = Vec::new();
    let mut buf = Vec::new();

    loop {
        let event = reader.read_event(&mut buf);

        match event {
            Ok(Event::Text(e)) => {
                texts.push(parse!(e.unescape_and_decode(reader))?);
            }

            Ok(Event::End(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                if starttag != tagname {
                    return Err(ParseError::new(anyhow!(
                        "Unmatched XML tags: <{}> .. <{}>",
                        starttag,
                        tagname
                    )));
                }

                let text = texts.join(" ").trim().to_string();
                texts.clear();

                return match starttag {
                    "undef" => Ok(LLSDValue::Undefined),

                    "real" => {
                        let text = if text.to_lowercase() == "nan" {
                            "NaN".to_string()
                        } else {
                            text
                        };

                        let value = parse!(text.parse::<f64>())?;
                        Ok(LLSDValue::Real(value))
                    }

                    "integer" => {
                        let value = parse_integer(&text)?;
                        Ok(LLSDValue::Integer(value))
                    }

                    "boolean" => {
                        let value = parse_boolean(&text)?;
                        Ok(LLSDValue::Boolean(value))
                    }

                    "string" => Ok(LLSDValue::String(text)),

                    "uri" => Ok(LLSDValue::String(text)),

                    "uuid" => {
                        let uuid = if text.is_empty() {
                            uuid::Uuid::nil()
                        } else {
                            parse!(uuid::Uuid::parse_str(&text))?
                        };

                        Ok(LLSDValue::UUID(uuid))
                    }

                    "date" => {
                        let value = parse_date(&text)?;
                        Ok(LLSDValue::Date(value))
                    }

                    "binary" => {
                        let value = parse_binary(&text, attrs)?;
                        Ok(LLSDValue::Binary(value))
                    }

                    _ => Err(ParseError::new(anyhow!(
                        "Unexpected primitive data type <{}> at position {}",
                        starttag,
                        reader.buffer_position()
                    ))),
                };
            }

            Ok(Event::Eof) => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected end of data in primitive value at position {}",
                    reader.buffer_position()
                )));
            }

            Ok(Event::Comment(_)) => {}

            Err(e) => {
                return Err(ParseError::new(anyhow!(
                    "Parse Error at position {}: {:?}",
                    reader.buffer_position(),
                    e
                )));
            }

            _ => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected parse event {:?} at position {} while parsing: {:?}",
                    event,
                    reader.buffer_position(),
                    starttag
                )));
            }
        }
    }
}

// Parse one map.
pub fn parse_map<R: BufRead>(reader: &mut Reader<&mut R>) -> Result<LLSDValue, ParseError> {
    let mut map: HashMap<String, LLSDValue> = HashMap::new();
    let mut texts = Vec::new();
    let mut buf = Vec::new();

    loop {
        let event = reader.read_event(&mut buf);

        match event {
            Ok(Event::Start(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                match tagname {
                    "key" => {
                        let (k, v) = parse_map_entry(reader)?;
                        let _dup = map.insert(k, v);
                    }

                    _ => {
                        return Err(ParseError::new(anyhow!(
                            "Expected 'key' in map, found '{}'",
                            tagname
                        )));
                    }
                }
            }

            Ok(Event::Text(e)) => {
                texts.push(parse!(e.unescape_and_decode(reader))?);
            }

            Ok(Event::End(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                if "map" != tagname {
                    return Err(ParseError::new(anyhow!(
                        "Unmatched XML tags: <{}> .. <{}>",
                        "map",
                        tagname
                    )));
                }

                return Ok(LLSDValue::Map(map));
            }

            Ok(Event::Eof) => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected end of data in map at position {}",
                    reader.buffer_position()
                )));
            }

            Ok(Event::Comment(_)) => {}

            Err(e) => {
                return Err(ParseError::new(anyhow!(
                    "Parse Error at position {}: {:?}",
                    reader.buffer_position(),
                    e
                )));
            }

            _ => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected parse event {:?} at position {} while parsing map",
                    event,
                    reader.buffer_position(),
                )));
            }
        }
    }
}

// Parse one map entry.
// Format <key> STRING </key> LLSDVALUE
pub fn parse_map_entry<R: BufRead>(
    reader: &mut Reader<&mut R>,
) -> Result<(String, LLSDValue), ParseError> {
    let mut texts = Vec::new();
    let mut buf = Vec::new();

    loop {
        let event = reader.read_event(&mut buf);

        match event {
            Ok(Event::Start(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                return Err(ParseError::new(anyhow!(
                    "Expected 'key' in map, found '{}'",
                    tagname
                )));
            }

            Ok(Event::Text(e)) => {
                texts.push(parse!(e.unescape_and_decode(reader))?);
            }

            Ok(Event::End(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                if "key" != tagname {
                    return Err(ParseError::new(anyhow!(
                        "Unmatched XML tags: <{}> .. <{}>",
                        "key",
                        tagname
                    )));
                }

                let mut buf = Vec::new();
                let k = texts.join(" ").trim().to_string();
                texts.clear();

                match reader.read_event(&mut buf) {
                    Ok(Event::Start(ref e)) => {
                        let tagname = parse!(std::str::from_utf8(e.name()))?;

                        let v = parse_value(reader, tagname, &e.attributes())?;

                        return Ok((k, v));
                    }

                    _ => {
                        return Err(ParseError::new(anyhow!(
                            "Unexpected parse error at position {} while parsing map entry",
                            reader.buffer_position()
                        )));
                    }
                }
            }

            Ok(Event::Eof) => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected end of data at position {}",
                    reader.buffer_position()
                )));
            }

            Ok(Event::Comment(_)) => {}

            Err(e) => {
                return Err(ParseError::new(anyhow!(
                    "Parse Error at position {}: {:?}",
                    reader.buffer_position(),
                    e
                )));
            }

            _ => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected parse event {:?} at position {} while parsing map entry",
                    event,
                    reader.buffer_position(),
                )));
            }
        }
    }
}

/// Parse one LLSD object. Recursive.
pub fn parse_array<R, F>(
    reader: &mut Reader<&mut R>,
    parse_value: F,
) -> Result<LLSDValue, ParseError>
where
    R: BufRead,
    F: Fn(&mut Reader<&mut R>, &str, &Attributes) -> Result<LLSDValue, ParseError>,
{
    let mut texts = Vec::new();
    let mut buf = Vec::new();
    let mut items: Vec<LLSDValue> = Vec::new();

    loop {
        let event = reader.read_event(&mut buf);

        match event {
            Ok(Event::Start(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                items.push(parse_value(reader, tagname, &e.attributes())?);
            }

            Ok(Event::Text(e)) => {
                texts.push(parse!(e.unescape_and_decode(reader))?);
            }

            Ok(Event::End(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                if "array" != tagname {
                    return Err(ParseError::new(anyhow!(
                        "Unmatched XML tags: <{}> .. <{}>",
                        "array",
                        tagname
                    )));
                }

                break;
            }

            Ok(Event::Eof) => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected end of data at position {}",
                    reader.buffer_position()
                )));
            }

            Ok(Event::Comment(_)) => {}

            Err(e) => {
                return Err(ParseError::new(anyhow!(
                    "Parse Error at position {}: {:?}",
                    reader.buffer_position(),
                    e
                )));
            }

            _ => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected parse event {:?} at position {} while parsing array",
                    event,
                    reader.buffer_position(),
                )));
            }
        }
    }

    Ok(LLSDValue::Array(items))
}

/// Parse binary object.
/// Input in base64, base16, or base85.
pub fn parse_binary(s: &str, attrs: &Attributes) -> Result<Vec<u8>, ParseError> {
    let encoding = match get_attr(attrs, b"encoding")? {
        Some(enc) => enc,
        None => "base64".to_string(),
    };

    Ok(match encoding.as_str() {
        "base64" => parse!(base64::engine::general_purpose::STANDARD.decode(s))?,

        "base16" => parse!(hex::decode(s))?,

        "base85" => match ascii85::decode(s) {
            Ok(v) => v,
            Err(e) => {
                return Err(ParseError::new(anyhow!("Base 85 decode error: {:?}", e)));
            }
        },

        _ => {
            return Err(ParseError::new(anyhow!(
                "Unknown encoding: <binary encoding=\"{}\">",
                encoding
            )));
        }
    })
}

/// Parse ISO 9660 date, simple form.
pub fn parse_date(s: &str) -> Result<i64, ParseError> {
    Ok(parse!(chrono::DateTime::parse_from_rfc3339(s))?.timestamp())
}

/// Parse integer. LSL allows the empty string as 0.
pub fn parse_integer(s: &str) -> Result<i32, ParseError> {
    let s = s.trim();

    if s.is_empty() {
        Ok(0)
    } else {
        parse!(s.parse::<i32>())
    }
}

/// Parse boolean. LSL allows 0. 0.0, false, 1. 1.0, true.
pub fn parse_boolean(s: &str) -> Result<bool, ParseError> {
    Ok(match s {
        "0" | "0.0" => false,
        "1" | "1.0" => true,
        _ => parse!(s.parse::<bool>())?,
    })
}

/// Search for attribute in attribute list.
fn get_attr(attrs: &Attributes, key: &[u8]) -> Result<Option<String>, ParseError> {
    for attr in attrs.clone() {
        let a = parse!(attr)?;

        if a.key != key {
            continue;
        }

        let v = parse!(a.unescaped_value())?;
        let sv = parse!(std::str::from_utf8(&v))?;

        return Ok(Some(sv.to_string()));
    }

    Ok(None)
}

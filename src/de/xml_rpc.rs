use crate::de::xml;
//
//  de/xml.rs -- XML-rpc deserializer for OpenSimulator's login
//
//  Library for serializing and de-serializing data in
//  Linden Lab Structured Data format.
//
//  XML-rpc format.
//
//  Benthicllsd
//  October, 2025.
//  License: LGPL.
//
use crate::errors::ParseError;
use crate::{parse, LLSDValue};
use anyhow::anyhow;
use quick_xml::events::attributes::Attributes;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
//  Constants
//
//
pub const XMLRPCPREFIX: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?><methodResponse>";
pub const XMLRPCPREFIX2: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<methodResponse>";
pub const XMLRPCPREFIX3: &str =
    "<?xml version=\\\"1.0\\\" encoding=\\\"utf-8\\\"?><methodResponse>";

pub fn from_str(xmlstr: &str) -> Result<LLSDValue, ParseError> {
    let parsed = from_reader(&mut BufReader::new(xmlstr.as_bytes()))?;
    let flattened = flatten_login_response(parsed);
    Ok(flattened)
}

fn flatten_login_response(value: LLSDValue) -> LLSDValue {
    match value {
        LLSDValue::Array(arr) => {
            let mut merged = HashMap::new();

            for v in arr {
                if let LLSDValue::Map(m) = v {
                    for (k, val) in m {
                        merged.insert(k, val);
                    }
                }
            }

            LLSDValue::Map(merged)
        }
        _ => value,
    }
}

/// Read XML from buffered source and parse into LLSDValue.
fn from_reader<R: BufRead>(rdr: &mut R) -> Result<LLSDValue, ParseError> {
    let mut reader = Reader::from_reader(rdr);
    reader.trim_text(true);
    reader.expand_empty_elements(true);

    let mut buf = Vec::new();
    let mut output: Option<LLSDValue> = None;

    loop {
        match reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name() {
                b"methodResponse" => {
                    if output.is_some() {
                        return Err(ParseError::new(anyhow!(
                            "More than one <methodResponse> block in data"
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
                                "Expected XMLRPC data, found {:?} error at position {}",
                                e.name(),
                                reader.buffer_position()
                            )));
                        }
                    }
                }

                _ => {
                    return Err(ParseError::new(anyhow!(
                        "Expected <methodResponse>, found {:?} error at position {}",
                        e.name(),
                        reader.buffer_position()
                    )));
                }
            },

            Ok(Event::Text(_)) => {}

            Ok(Event::End(_)) => {}

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
        "undef" | "real" | "integer" | "boolean" | "string" | "uri" | "binary" | "uuid" | "i4"
        | "date" => parse_primitive_value(reader, starttag, attrs),

        "map" => xml::parse_map(reader),

        "array" => xml::parse_array(reader, parse_value),

        "params" | "param" | "value" | "struct" | "data" => parse_container(reader, starttag),

        "member" => parse_member(reader),

        _ => Err(ParseError::new(anyhow!(
            "Unknown data type <{}> at position {}",
            starttag,
            reader.buffer_position()
        ))),
    }
}

fn parse_container<R: BufRead>(
    reader: &mut Reader<&mut R>,
    end_tag: &str,
) -> Result<LLSDValue, ParseError> {
    let mut items = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event(&mut buf) {
            Ok(Event::Start(e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                let val = parse_value(reader, tagname, &e.attributes())?;

                if !matches!(
                    val,
                    LLSDValue::Array(ref arr) if arr.is_empty()
                ) {
                    items.push(val);
                }
            }

            Ok(Event::End(e)) if e.name() == end_tag.as_bytes() => {
                break;
            }

            Ok(Event::Text(_)) | Ok(Event::Comment(_)) => {}

            Ok(Event::Eof) => {
                return Err(ParseError::new(anyhow!(
                    "Unexpected EOF while parsing <{}>",
                    end_tag
                )));
            }

            Err(e) => {
                return Err(ParseError::new(anyhow!(
                    "Parse error at {}: {:?}",
                    reader.buffer_position(),
                    e
                )));
            }

            _ => {}
        }

        buf.clear();
    }

    if items.is_empty() {
        Ok(LLSDValue::Array(vec![]))
    } else if items.len() == 1 {
        match &items[0] {
            LLSDValue::Map(_) => Ok(items[0].clone()),
            other => Ok(other.clone()),
        }
    } else {
        let mut merged_items = Vec::new();

        for item in items {
            match item {
                LLSDValue::Array(arr) => {
                    if arr.iter().all(|v| matches!(v, LLSDValue::Map(_))) {
                        let mut merged_map = HashMap::new();

                        for v in arr {
                            if let LLSDValue::Map(m) = v {
                                for (k, val) in m {
                                    merged_map.insert(k, val);
                                }
                            }
                        }

                        merged_items.push(LLSDValue::Map(merged_map));
                    } else {
                        merged_items.push(LLSDValue::Array(arr));
                    }
                }

                LLSDValue::Map(_) => {
                    merged_items.push(item);
                }

                other => {
                    merged_items.push(other);
                }
            }
        }

        Ok(LLSDValue::Array(merged_items))
    }
}

/// Parse one map.
fn parse_member<R: BufRead>(reader: &mut Reader<&mut R>) -> Result<LLSDValue, ParseError> {
    let mut map: HashMap<String, LLSDValue> = HashMap::new();
    let mut texts = Vec::new();
    let mut buf = Vec::new();

    loop {
        let event = reader.read_event(&mut buf);

        match event {
            Ok(Event::Start(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                match tagname {
                    "name" => {
                        let (k, v) = parse_member_entry(reader)?;
                        let _dup = map.insert(k, v);
                    }

                    _ => {
                        return Err(ParseError::new(anyhow!(
                            "Expected 'name' in map, found '{}'",
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

                if "member" != tagname {
                    return Err(ParseError::new(anyhow!(
                        "Unmatched XML tags: <{}> .. <{}>",
                        "member",
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
                    reader.buffer_position()
                )));
            }
        }
    }
}

/// Parse one map entry.
///
/// Format:
/// `<name> STRING </name> LLSDVALUE`
fn parse_member_entry<R: BufRead>(
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
                    "Expected 'name' in map, found '{}'",
                    tagname
                )));
            }

            Ok(Event::Text(e)) => {
                texts.push(parse!(e.unescape_and_decode(reader))?);
            }

            Ok(Event::End(ref e)) => {
                let tagname = parse!(std::str::from_utf8(e.name()))?;

                if "name" != tagname {
                    return Err(ParseError::new(anyhow!(
                        "Unmatched XML tags: <{}> .. <{}>",
                        "name",
                        tagname
                    )));
                }

                let k = texts.join(" ").trim().to_string();
                texts.clear();

                let mut buf = Vec::new();

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
                    reader.buffer_position()
                )));
            }
        }
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
                        let value = xml::parse_integer(&text)?;
                        Ok(LLSDValue::Integer(value))
                    }

                    "boolean" => {
                        let value = xml::parse_boolean(&text)?;
                        Ok(LLSDValue::Boolean(value))
                    }

                    "string" => match text.to_lowercase().as_str() {
                        "true" => Ok(LLSDValue::Boolean(true)),

                        "false" => Ok(LLSDValue::Boolean(false)),

                        _ => {
                            if let Ok(uuid) = uuid::Uuid::parse_str(&text) {
                                Ok(LLSDValue::UUID(uuid))
                            } else {
                                Ok(LLSDValue::String(text))
                            }
                        }
                    },

                    "uri" => Ok(LLSDValue::String(text)),

                    "uuid" => {
                        let uuid = if text.is_empty() {
                            uuid::Uuid::nil()
                        } else {
                            parse!(uuid::Uuid::parse_str(&text))?
                        };

                        Ok(LLSDValue::UUID(uuid))
                    }

                    "i4" => {
                        let value = xml::parse_integer(&text)?;
                        Ok(LLSDValue::Integer(value))
                    }

                    "date" => {
                        let value = xml::parse_date(&text)?;
                        Ok(LLSDValue::Date(value))
                    }

                    "binary" => {
                        let value = xml::parse_binary(&text, attrs)?;
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

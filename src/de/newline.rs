use std::collections::HashMap;

use anyhow::anyhow;
use uuid::Uuid;

use crate::{errors::ParseError, parse, LLSDValue};

pub const LLSDNEWLINEPREFIX: &str = "LLWearable version 22";

/// parsing information for newline separated llsd structs.
/// almost completely undocumented.
pub fn parse_llwearable_to_llsd(text: &str) -> Result<LLSDValue, ParseError> {
    let mut lines = text.lines().map(str::trim).peekable();

    let mut get_line = || {
        lines
            .next()
            .ok_or_else(|| anyhow!("Unexpected end of input"))
    };

    let mut root_map = HashMap::new();

    // Version
    let version_line = parse!(get_line())?;

    let version = parse!(version_line
        .strip_prefix("LLWearable version ")
        .ok_or_else(|| anyhow!("Expected 'LLWearable version N'")))?;

    let version = parse!(version.parse::<i32>())?;

    root_map.insert("version".to_string(), LLSDValue::Integer(version));

    // Name
    let name = parse!(get_line())?;

    root_map.insert("name".to_string(), LLSDValue::String(name.to_string()));

    let mut permissions = HashMap::new();
    let mut sale_info = HashMap::new();
    let mut parameters = HashMap::new();
    let mut textures = HashMap::new();

    while let Ok(line) = get_line() {
        let line = line.trim();

        if line.starts_with("permissions") {
            let brace = parse!(get_line())?;

            if brace != "{" {
                return Err(ParseError::new(anyhow!("Expected '{{' after permissions")));
            }

            while let Ok(l) = get_line() {
                let l = l.trim();

                if l == "}" {
                    break;
                }

                let parts: Vec<_> = l.split_whitespace().collect();

                if parts.len() == 2 {
                    let val = if parts[1].contains('-') {
                        LLSDValue::UUID(parse!(Uuid::parse_str(parts[1]))?)
                    } else {
                        LLSDValue::Integer(parse!(i32::from_str_radix(parts[1], 16))?)
                    };

                    permissions.insert(parts[0].to_string(), val);
                }
            }
        } else if line.starts_with("sale_info") {
            let brace = parse!(get_line())?;

            if brace != "{" {
                return Err(ParseError::new(anyhow!("Expected '{{' after sale_info")));
            }

            while let Ok(l) = get_line() {
                let l = l.trim();

                if l == "}" {
                    break;
                }

                let parts: Vec<_> = l.split_whitespace().collect();

                if parts.len() != 2 {
                    return Err(ParseError::new(anyhow!(
                        "Invalid sale_info line format: {}",
                        l
                    )));
                }

                let key = parts[0];
                let value = parts[1];

                match key {
                    "sale_type" => {
                        sale_info.insert(key.to_string(), LLSDValue::String(value.to_string()));
                    }

                    "sale_price" => {
                        let price = parse!(value.parse::<i32>().map_err(|_| {
                            anyhow!("Invalid integer for sale_price: {}", value)
                        }))?;

                        sale_info.insert(key.to_string(), LLSDValue::Integer(price));
                    }

                    _ => {
                        return Err(ParseError::new(anyhow!(
                            "Unexpected field in sale_info: {}",
                            key
                        )));
                    }
                }
            }
        } else if line.starts_with("parameters") {
            let parts: Vec<_> = line.split_whitespace().collect();

            let count = parse!(parts
                .get(1)
                .ok_or_else(|| anyhow!("Missing parameter count")))?;

            let count = parse!(count.parse::<usize>())?;

            for _ in 0..count {
                let param_line = parse!(get_line())?;

                let parts: Vec<_> = param_line.split_whitespace().collect();

                if parts.len() == 2 {
                    let key = parse!(parts[0].parse::<f64>())?;
                    let val = parse!(parts[1].parse::<f64>())?;

                    parameters.insert(key.to_string(), LLSDValue::Real(val));
                }
            }
        } else if line.starts_with("textures") {
            let parts: Vec<_> = line.split_whitespace().collect();

            let count = parse!(parts.get(1).ok_or_else(|| anyhow!("Missing texture count")))?;

            let count = parse!(count.parse::<usize>())?;

            for _ in 0..count {
                let tex_line = parse!(get_line())?;

                let parts: Vec<_> = tex_line.split_whitespace().collect();

                if parts.len() == 2 {
                    textures.insert(
                        parts[0].to_string(),
                        LLSDValue::UUID(parse!(Uuid::parse_str(parts[1]))?),
                    );
                }
            }
        }
    }

    root_map.insert("permissions".to_string(), LLSDValue::Map(permissions));

    root_map.insert("sale_info".to_string(), LLSDValue::Map(sale_info));

    root_map.insert("parameters".to_string(), LLSDValue::Map(parameters));

    root_map.insert("textures".to_string(), LLSDValue::Map(textures));

    Ok(LLSDValue::Map(root_map))
}

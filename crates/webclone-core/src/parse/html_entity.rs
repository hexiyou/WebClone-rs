//! 极简 HTML 实体解码。只处理跟 URL 相关的那几个实体，
//! 属性里的 `&amp;` 不解出来会导致 URL 里的查询参数被截断。

/// 解码实体。不属于已知实体时返回 None。
fn decode_entity(entity: &[u8]) -> Option<String> {
    if entity.is_empty() {
        return None;
    }

    if entity[0] == b'#' {
        if entity.len() < 2 {
            return None;
        }
        let (is_hex, digits) = if entity[1] == b'x' || entity[1] == b'X' {
            (true, &entity[2..])
        } else {
            (false, &entity[1..])
        };
        if digits.is_empty() {
            return None;
        }
        let text = std::str::from_utf8(digits).ok()?;
        let code = if is_hex {
            u32::from_str_radix(text, 16).ok()?
        } else {
            text.parse::<u32>().ok()?
        };
        if code > 0x10FFFF {
            return None;
        }
        return char::from_u32(code).map(|c| c.to_string());
    }

    match entity {
        b"amp" => Some("&".to_owned()),
        b"lt" => Some("<".to_owned()),
        b"gt" => Some(">".to_owned()),
        b"quot" => Some("\"".to_owned()),
        b"apos" => Some("'".to_owned()),
        b"nbsp" => Some(" ".to_owned()),
        _ => None,
    }
}

pub fn decode(input: &str) -> String {
    let bytes = input.as_bytes();
    if !bytes.contains(&b'&') {
        return input.to_owned();
    }

    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c != b'&' {
            // 按字符推进，保证多字节字符不被切开
            let ch = input[i..].chars().next().expect("下标必然落在字符边界");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        let semi = match bytes[i + 1..].iter().position(|b| *b == b';') {
            Some(p) => i + 1 + p,
            None => {
                out.push('&');
                i += 1;
                continue;
            }
        };

        if semi - i > 12 {
            out.push('&');
            i += 1;
            continue;
        }

        match decode_entity(&bytes[i + 1..semi]) {
            Some(decoded) => {
                out.push_str(&decoded);
                i = semi + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }

    out
}

/// 写回属性时把 & 重新编码，避免生成不合法的 HTML。
pub fn encode_for_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
}

//! Byte-exact tests of the lobby messages. The expected bytes are written
//! out by hand from the layouts in `docs/notes/` (and the
//! disassembly they cite), never taken from the writers.

use super::ServerMessage;

mod model;
mod requests;
mod writers;

/// Bytes from hex, line by line. A token of upper-case hex digits is data;
/// `"text"` is its ASCII bytes and `'text'` the same with its `u8` length
/// first (a wire string); `|` is ignored. The first other token ends the
/// line: the rest is a comment.
fn h(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut chars = line.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '|' {
                chars.next();
            } else if c == '"' || c == '\'' {
                chars.next();
                let literal: Vec<u8> = chars
                    .by_ref()
                    .take_while(|&inner| inner != c)
                    .map(|inner| inner as u8)
                    .collect();
                if c == '\'' {
                    out.push(literal.len() as u8);
                }
                out.extend(literal);
            } else {
                let mut token = String::new();
                while let Some(&inner) = chars.peek() {
                    if inner.is_whitespace() {
                        break;
                    }
                    token.push(inner);
                    chars.next();
                }
                let is_data = token.len().is_multiple_of(2)
                    && token
                        .chars()
                        .all(|digit| matches!(digit, '0'..='9' | 'A'..='F'));
                if !is_data {
                    break;
                }
                for pair in token.as_bytes().chunks(2) {
                    let pair = std::str::from_utf8(pair).unwrap();
                    out.push(u8::from_str_radix(pair, 16).unwrap());
                }
            }
        }
    }
    out
}

/// The message as `id, body` (no transport `x` byte).
fn bytes(message: &ServerMessage) -> Vec<u8> {
    let mut out = vec![message.id_byte()];
    out.extend_from_slice(message.body());
    out
}

#[track_caller]
fn assert_bytes(message: &ServerMessage, expected: &str) {
    assert_eq!(bytes(message), h(expected), "{:?}", message.id());
}

#[test]
fn the_hex_helper_reads_strings() {
    assert_eq!(h("01 'ab' \"c\" FF"), [1, 2, b'a', b'b', b'c', 0xFF]);
    assert_eq!(
        h("0102 a comment 03
 'A b' | 04   FF is said"),
        [1, 2, 3, b'A', b' ', b'b', 4, 0xFF]
    );
}

//! Aides MIME partagées par la lecture OpenPGP et S/MIME.

use mail_parser::MessageParser;

/// Mêmes octets avec des fins de ligne CRLF, comme les signe un client (RFC 3156 §5) : le transport peut les avoir normalisées.
pub fn crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 40);
    let mut prev = 0u8;
    for &b in data {
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        prev = b;
    }
    out
}

pub fn text_of(data: &[u8], mime: bool) -> (Option<String>, Option<String>) {
    if mime {
        if let Some(m) = MessageParser::default().parse(data) {
            return (
                m.body_text(0).map(|c| c.into_owned()),
                m.body_html(0).map(|c| c.into_owned()),
            );
        }
    }
    (Some(String::from_utf8_lossy(data).into_owned()), None)
}

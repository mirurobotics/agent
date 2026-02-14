// internal crates
use crate::crypt::errors::{Base64DecodeErr, ConvertBytesToStringErr, CryptErr};
use crate::trace;
// external crates
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};

// encode bytes using the specified base64 method
pub fn encode_bytes(
    bytes: &[u8],
    method: base64::engine::general_purpose::GeneralPurpose,
) -> String {
    method.encode(bytes)
}

pub fn encode_bytes_standard(bytes: &[u8]) -> String {
    encode_bytes(bytes, STANDARD)
}

pub fn encode_bytes_url_safe_no_pad(bytes: &[u8]) -> String {
    encode_bytes(bytes, URL_SAFE_NO_PAD)
}

pub fn encode_bytes_url_safe(bytes: &[u8]) -> String {
    encode_bytes(bytes, URL_SAFE)
}

// encode a string using the specified base64 method
pub fn encode_string(
    string: &str,
    method: base64::engine::general_purpose::GeneralPurpose,
) -> String {
    encode_bytes(string.as_bytes(), method)
}

pub fn encode_string_standard(string: &str) -> String {
    encode_string(string, STANDARD)
}

pub fn encode_string_url_safe_no_pad(string: &str) -> String {
    encode_string(string, URL_SAFE_NO_PAD)
}

pub fn encode_string_url_safe(string: &str) -> String {
    encode_string(string, URL_SAFE)
}

pub fn decode_bytes(
    token: &str,
    method: base64::engine::general_purpose::GeneralPurpose,
) -> Result<Vec<u8>, CryptErr> {
    method.decode(token.as_bytes()).map_err(|e| {
        CryptErr::Base64DecodeErr(Base64DecodeErr {
            source: e,
            trace: trace!(),
        })
    })
}

pub fn decode_bytes_standard(token: &str) -> Result<Vec<u8>, CryptErr> {
    decode_bytes(token, STANDARD)
}

pub fn decode_bytes_url_safe_no_pad(token: &str) -> Result<Vec<u8>, CryptErr> {
    decode_bytes(token, URL_SAFE_NO_PAD)
}

pub fn decode_bytes_url_safe(token: &str) -> Result<Vec<u8>, CryptErr> {
    decode_bytes(token, URL_SAFE)
}

pub fn decode_string(
    token: &str,
    method: base64::engine::general_purpose::GeneralPurpose,
) -> Result<String, CryptErr> {
    let bytes = decode_bytes(token, method)?;
    let string = String::from_utf8(bytes).map_err(|e| {
        CryptErr::ConvertBytesToStringErr(ConvertBytesToStringErr {
            source: e,
            trace: trace!(),
        })
    })?;
    Ok(string)
}

pub fn decode_string_standard(token: &str) -> Result<String, CryptErr> {
    decode_string(token, STANDARD)
}

pub fn decode_string_url_safe_no_pad(token: &str) -> Result<String, CryptErr> {
    decode_string(token, URL_SAFE_NO_PAD)
}

pub fn decode_string_url_safe(token: &str) -> Result<String, CryptErr> {
    decode_string(token, URL_SAFE)
}

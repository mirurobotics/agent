// internal crates
use miru_agent::crypt::base64;
use miru_agent::crypt::errors::CryptErr;

pub mod encode_bytes_standard {
    use super::*;

    #[test]
    fn roundtrip() {
        let input = b"hello world";
        let encoded = base64::encode_bytes_standard(input);
        let decoded = base64::decode_bytes_standard(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn known_value() {
        let encoded = base64::encode_bytes_standard(b"hello world");
        assert_eq!(encoded, "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn empty_input() {
        let encoded = base64::encode_bytes_standard(b"");
        assert_eq!(encoded, "");
        let decoded = base64::decode_bytes_standard(&encoded).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn binary_data() {
        let input: Vec<u8> = (0..=255).collect();
        let encoded = base64::encode_bytes_standard(&input);
        let decoded = base64::decode_bytes_standard(&encoded).unwrap();
        assert_eq!(decoded, input);
    }
}

pub mod encode_bytes_url_safe {
    use super::*;

    #[test]
    fn roundtrip() {
        let input = b"hello world";
        let encoded = base64::encode_bytes_url_safe(input);
        let decoded = base64::decode_bytes_url_safe(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn no_plus_or_slash() {
        // bytes that produce + and / in standard base64
        let input = b"\xfb\xff\xfe";
        let encoded = base64::encode_bytes_url_safe(input);
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
    }
}

pub mod encode_bytes_url_safe_no_pad {
    use super::*;

    #[test]
    fn roundtrip() {
        let input = b"hello world";
        let encoded = base64::encode_bytes_url_safe_no_pad(input);
        let decoded = base64::decode_bytes_url_safe_no_pad(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn no_padding() {
        let encoded = base64::encode_bytes_url_safe_no_pad(b"hello world");
        assert!(!encoded.contains('='));
    }
}

pub mod encode_string_standard {
    use super::*;

    #[test]
    fn roundtrip() {
        let input = "hello world";
        let encoded = base64::encode_string_standard(input);
        let decoded = base64::decode_string_standard(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn known_value() {
        let encoded = base64::encode_string_standard("hello world");
        assert_eq!(encoded, "aGVsbG8gd29ybGQ=");
    }
}

pub mod encode_string_url_safe {
    use super::*;

    #[test]
    fn roundtrip() {
        let input = "hello world";
        let encoded = base64::encode_string_url_safe(input);
        let decoded = base64::decode_string_url_safe(&encoded).unwrap();
        assert_eq!(decoded, input);
    }
}

pub mod encode_string_url_safe_no_pad {
    use super::*;

    #[test]
    fn roundtrip() {
        let input = "hello world";
        let encoded = base64::encode_string_url_safe_no_pad(input);
        let decoded = base64::decode_string_url_safe_no_pad(&encoded).unwrap();
        assert_eq!(decoded, input);
    }
}

pub mod decode_bytes_standard {
    use super::*;

    #[test]
    fn known_value() {
        let decoded = base64::decode_bytes_standard("aGVsbG8gd29ybGQ=").unwrap();
        assert_eq!(decoded, b"hello world");
    }

    #[test]
    fn invalid_base64() {
        let result = base64::decode_bytes_standard("!!!invalid!!!");
        assert!(matches!(result, Err(CryptErr::Base64DecodeErr { .. })));
    }
}

pub mod decode_bytes_url_safe {
    use super::*;

    #[test]
    fn known_value() {
        let decoded = base64::decode_bytes_url_safe("aGVsbG8gd29ybGQ=").unwrap();
        assert_eq!(decoded, b"hello world");
    }

    #[test]
    fn invalid_base64() {
        let result = base64::decode_bytes_url_safe("!!!invalid!!!");
        assert!(matches!(result, Err(CryptErr::Base64DecodeErr { .. })));
    }
}

pub mod decode_bytes_url_safe_no_pad {
    use super::*;

    #[test]
    fn known_value() {
        let decoded = base64::decode_bytes_url_safe_no_pad("aGVsbG8gd29ybGQ").unwrap();
        assert_eq!(decoded, b"hello world");
    }

    #[test]
    fn invalid_base64() {
        let result = base64::decode_bytes_url_safe_no_pad("!!!invalid!!!");
        assert!(matches!(result, Err(CryptErr::Base64DecodeErr { .. })));
    }
}

pub mod decode_string_standard {
    use super::*;

    #[test]
    fn known_value() {
        let decoded = base64::decode_string_standard("aGVsbG8gd29ybGQ=").unwrap();
        assert_eq!(decoded, "hello world");
    }

    #[test]
    fn invalid_base64() {
        let result = base64::decode_string_standard("!!!invalid!!!");
        assert!(matches!(result, Err(CryptErr::Base64DecodeErr { .. })));
    }

    #[test]
    fn invalid_utf8() {
        let invalid_utf8: &[u8] = &[0xFF, 0xFE, 0xFD];
        let encoded = base64::encode_bytes_standard(invalid_utf8);
        let result = base64::decode_string_standard(&encoded);
        assert!(matches!(
            result,
            Err(CryptErr::ConvertBytesToStringErr { .. })
        ));
    }
}

pub mod decode_string_url_safe {
    use super::*;

    #[test]
    fn known_value() {
        let decoded = base64::decode_string_url_safe("aGVsbG8gd29ybGQ=").unwrap();
        assert_eq!(decoded, "hello world");
    }

    #[test]
    fn invalid_utf8() {
        let invalid_utf8: &[u8] = &[0xFF, 0xFE, 0xFD];
        let encoded = base64::encode_bytes_url_safe(invalid_utf8);
        let result = base64::decode_string_url_safe(&encoded);
        assert!(matches!(
            result,
            Err(CryptErr::ConvertBytesToStringErr { .. })
        ));
    }
}

pub mod decode_string_url_safe_no_pad {
    use super::*;

    #[test]
    fn known_value() {
        let decoded = base64::decode_string_url_safe_no_pad("aGVsbG8gd29ybGQ").unwrap();
        assert_eq!(decoded, "hello world");
    }

    #[test]
    fn invalid_utf8() {
        let invalid_utf8: &[u8] = &[0xFF, 0xFE, 0xFD];
        let encoded = base64::encode_bytes_url_safe_no_pad(invalid_utf8);
        let result = base64::decode_string_url_safe_no_pad(&encoded);
        assert!(matches!(
            result,
            Err(CryptErr::ConvertBytesToStringErr { .. })
        ));
    }
}

pub mod cross_method {
    use super::*;

    #[test]
    fn standard_and_url_safe_differ_for_special_bytes() {
        // bytes 0xFB, 0xFF, 0xFE produce +/= in standard but -/_ in url-safe
        let input = b"\xfb\xff\xfe";
        let standard = base64::encode_bytes_standard(input);
        let url_safe = base64::encode_bytes_url_safe(input);
        assert_ne!(standard, url_safe);
    }
}

// internal crates
use miru_agent::crypt::base64;
use miru_agent::crypt::errors::CryptErr;
use miru_agent::crypt::jwt;
use miru_agent::crypt::jwt::Claims;

// external crates
use chrono::Utc;
use serde_json::json;

pub mod decode {
    use super::*;

    #[test]
    fn invalid_jwt_format() {
        let cases = vec![
            ("", "empty string"),
            ("single_part", "one part"),
            ("two.parts", "two parts"),
            ("eyJ.eyJ.sig.extra", "four parts"),
        ];
        for (token, label) in cases {
            let result = jwt::decode(token);
            assert!(
                matches!(result, Err(CryptErr::InvalidJWTErr { .. })),
                "expected InvalidJWTErr for {label}, got {result:?}"
            );
        }
    }

    #[test]
    fn payload_not_decodable() {
        let payload = "arglechargle";
        let token = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{payload}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
        );
        let result = jwt::decode(&token);
        println!("Result: {result:?}");
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(CryptErr::ConvertBytesToStringErr { .. })
        ));
    }

    #[test]
    fn invalid_payload_format() {
        // missing the issuer
        let invalid_payloads = vec![
            json!({
                // missing the issuer
                "aud": "device",
                "exp": 1721517034,
                "iat": 1721495434,
                "sub": "75899aa4-b08a-4047-8526-880b1b832973"
            })
            .to_string(),
            json!({
                // missing the audience
                "iss": "miru",
                "exp": 1721517034,
                "iat": 1721495434,
                "sub": "75899aa4-b08a-4047-8526-880b1b832973"
            })
            .to_string(),
            json!({
                // missing the subject
                "iss": "miru",
                "aud": "device",
                "exp": 1721517034,
                "iat": 1721495434,
            })
            .to_string(),
            json!({
                // missing the expiration time
                "iss": "miru",
                "aud": "device",
                "iat": 1721495434,
                "sub": "75899aa4-b08a-4047-8526-880b1b832973"
            })
            .to_string(),
            json!({
                // missing the issued at time
                "iss": "miru",
                "aud": "device",
                "exp": 1721517034,
                "sub": "75899aa4-b08a-4047-8526-880b1b832973"
            })
            .to_string(),
        ];

        for payload in invalid_payloads {
            let token = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
            base64::encode_string_url_safe_no_pad(&payload)
            );
            let result = jwt::decode(&token);
            assert!(result.is_err());
            assert!(matches!(result, Err(CryptErr::InvalidJWTPayloadErr { .. })));
        }
    }

    #[test]
    fn success() {
        let payload = json!({
            "iss": "miru",
            "aud": "device",
            "exp": 1721517034,
            "iat": 1721495434,
            "sub": "75899aa4-b08a-4047-8526-880b1b832973"
        })
        .to_string();

        let token = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
            base64::encode_string_url_safe_no_pad(&payload)
        );
        let claims = jwt::decode(&token).unwrap();
        let expected = Claims {
            iss: "miru".to_string(),
            aud: "device".to_string(),
            exp: 1721517034,
            iat: 1721495434,
            sub: "75899aa4-b08a-4047-8526-880b1b832973".to_string(),
        };
        assert_eq!(claims, expected);
    }
}

pub mod extract_device_id {
    use super::*;

    #[test]
    fn payload_not_decodable() {
        let payload = "arglechargle";
        let token = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{payload}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
        );
        let result = jwt::extract_device_id(&token).unwrap_err();
        println!("Result: {result:?}");
        assert!(matches!(result, CryptErr::ConvertBytesToStringErr { .. }));
    }

    #[test]
    fn success() {
        let payload = json!({
            "iss": "miru",
            "aud": "device",
            "exp": 1721517034,
            "iat": 1721495434,
            "sub": "75899aa4-b08a-4047-8526-880b1b832973"
        })
        .to_string();

        let token = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
            base64::encode_string_url_safe_no_pad(&payload)
        );
        let device_id = jwt::extract_device_id(&token).unwrap();
        assert_eq!(device_id, "75899aa4-b08a-4047-8526-880b1b832973");
    }
}

pub mod validate_claims {
    use super::*;

    #[test]
    fn device_claims_invalid() {
        let now = Utc::now().timestamp();
        let invalid_claims = vec![
            // issuer isn't miru
            Claims {
                iss: "Uncle Sam".to_string(),
                aud: "device".to_string(),
                iat: now,
                exp: now + 1000,
                sub: "75899aa4-b08a-4047-8526-880b1b832973".to_string(),
            },
            // audience isn't device
            Claims {
                iss: "miru".to_string(),
                aud: "user".to_string(),
                iat: now,
                exp: now + 1000,
                sub: "75899aa4-b08a-4047-8526-880b1b832973".to_string(),
            },
            // issued at time is in the future
            Claims {
                iss: "miru".to_string(),
                aud: "device".to_string(),
                iat: now + 1000,
                exp: now + 1000,
                sub: "75899aa4-b08a-4047-8526-880b1b832973".to_string(),
            },
            // expiration time is in the past
            Claims {
                iss: "miru".to_string(),
                aud: "device".to_string(),
                iat: now,
                exp: now - 1,
                sub: "75899aa4-b08a-4047-8526-880b1b832973".to_string(),
            },
        ];
        for claim in invalid_claims {
            let result = jwt::validate_claims(claim);
            assert!(result.is_err());
            assert!(matches!(result, Err(CryptErr::InvalidJWTErr { .. })));
        }
    }

    #[test]
    fn device_claims_valid() {
        let now = Utc::now().timestamp();
        let claim = Claims {
            iss: "miru".to_string(),
            aud: "device".to_string(),
            iat: now,
            exp: now + 1000,
            sub: "75899aa4-b08a-4047-8526-880b1b832973".to_string(),
        };
        let device_id = jwt::validate_claims(claim).unwrap();
        assert_eq!(device_id, "75899aa4-b08a-4047-8526-880b1b832973");
    }

    #[test]
    fn iat_within_tolerance_is_valid() {
        let now = Utc::now().timestamp();
        // iat 10 seconds in the future is within the 15-second tolerance
        let claim = Claims {
            iss: "miru".to_string(),
            aud: "device".to_string(),
            iat: now + 10,
            exp: now + 1000,
            sub: "device-1".to_string(),
        };
        assert!(jwt::validate_claims(claim).is_ok());
    }
}

pub mod validate {
    use super::*;

    // NOTE: jwt::validate performs claims validation only (issuer, audience, expiry, iat).
    // It does NOT verify the cryptographic signature — that is the backend's responsibility.
    // The fabricated signature in these tests is therefore intentional.

    #[test]
    fn success() {
        let now = Utc::now().timestamp();
        let payload = json!({
            "iss": "miru",
            "aud": "device",
            "exp": now + 1000,
            "iat": now,
            "sub": "75899aa4-b08a-4047-8526-880b1b832973"
        })
        .to_string();

        let token = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
            base64::encode_string_url_safe_no_pad(&payload)
        );
        let device_id = jwt::validate(&token).unwrap();
        assert_eq!(device_id, "75899aa4-b08a-4047-8526-880b1b832973");
    }

    #[test]
    fn expired_token() {
        let now = Utc::now().timestamp();
        let payload = json!({
            "iss": "miru",
            "aud": "device",
            "exp": now - 100,
            "iat": now - 200,
            "sub": "75899aa4-b08a-4047-8526-880b1b832973"
        })
        .to_string();

        let token = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
            base64::encode_string_url_safe_no_pad(&payload)
        );
        let result = jwt::validate(&token);
        assert!(matches!(result, Err(CryptErr::InvalidJWTErr { .. })));
    }

    #[test]
    fn invalid_format() {
        let result = jwt::validate("not.a.valid-token");
        assert!(result.is_err());
    }
}

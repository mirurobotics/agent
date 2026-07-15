// internal crates
use miru_agent::authn::token::{Token, Updates};
use miru_agent::filesys::{
    dirs, files,
    state_file::{ConcurrentStateFile, Options, SingleThreadStateFile},
    FileSysErr, PathExt, WriteOptions,
};

// external crates
use chrono::{Duration, Utc};
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

// ========================= SINGLE THREADED STATE FILE =========================== //
type SingleThreadTokenFile = SingleThreadStateFile<Token, Updates>;

// Options that create the file with the default token if it is absent/unreadable.
fn opts_default() -> Options<Token> {
    Options {
        default: Some(Token::default()),
        mode: None,
    }
}

pub mod open_read_only {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        let result = SingleThreadTokenFile::open(file, Options::default()).await;
        assert!(matches!(result, Err(FileSysErr::PathDoesNotExistErr(_))));
    }

    #[tokio::test]
    async fn exists_invalid_data() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        files::write_string(&file, "invalid-data", WriteOptions::default())
            .await
            .unwrap();

        // ensure the contents is correct
        let result = SingleThreadTokenFile::open(file, Options::default()).await;
        assert!(matches!(result, Err(FileSysErr::ParseJSONErr(_))));
    }

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        files::write_json(&file, &token, WriteOptions::default())
            .await
            .unwrap();

        // ensure the contents is correct
        let state_file = SingleThreadTokenFile::open(file, Options::default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &token);
    }
}

pub mod open_with_default {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let state_file = SingleThreadTokenFile::open(file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
        // the default was persisted to disk
        assert!(file.exists());
    }

    #[tokio::test]
    async fn exists_invalid_data() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file with invalid data — the read fails, so the default is
        // written and the file is left holding valid JSON
        files::write_string(&file, "invalid-data", WriteOptions::default())
            .await
            .unwrap();

        let state_file = SingleThreadTokenFile::open(file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());

        // reopening read-only now succeeds because the file holds valid JSON
        let reopened = SingleThreadTokenFile::open(file, Options::default())
            .await
            .unwrap();
        assert_eq!(reopened.read().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        files::write_json(&file, &token, WriteOptions::default())
            .await
            .unwrap();

        // an existing readable file is loaded as-is; the default is ignored
        let state_file = SingleThreadTokenFile::open(file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &token);
    }

    // When the read fails and the create write also fails, the write error
    // propagates instead of being swallowed. A regular file standing in for the
    // parent directory makes the create fail without relying on permissions.
    #[tokio::test]
    async fn create_failure_propagates() {
        let dir = dirs::temp("testing").unwrap();
        let blocker = dir.file("blocker");
        files::write_string(&blocker, "x", WriteOptions::default())
            .await
            .unwrap();

        // `blocker` is a file, so its "child" has no real parent directory
        let file = dir.file("blocker/child.json");
        let result = SingleThreadTokenFile::open(file, opts_default()).await;
        assert!(result.is_err());
    }
}

pub mod read {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let state_file = SingleThreadTokenFile::open(file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        let state_file = SingleThreadTokenFile::open(file.clone(), opts_default())
            .await
            .unwrap();

        // delete the file
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // should still be able to read the file since it's cached in memory
        assert_eq!(state_file.read().as_ref(), &Token::default());
    }
}

pub mod write {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        let mut state_file = SingleThreadTokenFile::open(file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());

        // write to the file
        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        state_file.write(token.clone()).await.unwrap();
        assert_eq!(state_file.read().as_ref(), &token);
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        let mut state_file = SingleThreadTokenFile::open(file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());

        // delete the file
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // write to the file
        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        state_file.write(token.clone()).await.unwrap();
        assert_eq!(state_file.read().as_ref(), &token);
    }
}

pub mod patch {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let mut state_file = SingleThreadTokenFile::open(file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());

        // patch the file
        let updates = Updates {
            token: Some("test-token".to_string()),
            expires_at: Some(Utc::now() + Duration::days(1)),
        };
        let expected = Token {
            token: updates.token.clone().unwrap(),
            expires_at: updates.expires_at.unwrap(),
        };
        state_file.patch(updates).await.unwrap();
        assert_eq!(&expected, state_file.read().as_ref());
    }

    #[tokio::test]
    async fn no_op_skips_write() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        let mut state_file = SingleThreadTokenFile::open(
            file.clone(),
            Options {
                default: Some(token.clone()),
                mode: None,
            },
        )
        .await
        .unwrap();

        // delete the backing file so any real write would fail
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // patch with empty updates — merge produces no change, so write is skipped
        state_file.patch(Updates::empty()).await.unwrap();
        assert_eq!(state_file.read().as_ref(), &token);

        // backing file should still not exist (no write occurred)
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let mut state_file = SingleThreadTokenFile::open(file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());

        // delete the file
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // patch the file
        let updates = Updates {
            token: Some("test-token".to_string()),
            expires_at: Some(Utc::now() + Duration::days(1)),
        };
        let expected = Token {
            token: updates.token.clone().unwrap(),
            expires_at: updates.expires_at.unwrap(),
        };
        state_file.patch(updates).await.unwrap();
        assert_eq!(&expected, state_file.read().as_ref());
    }
}

#[cfg(unix)]
pub mod mode {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // A state file opened with `mode: Some(0o600)` restricts the backing file to
    // owner read/write on both the initial create and every subsequent write —
    // this is how the auth token (a live bearer credential) is protected at rest.
    #[tokio::test]
    async fn restricts_permissions_on_create_and_write() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("token.json");

        // create path: file does not exist yet, so `open` writes the default at
        // 0o600
        let mut state_file = SingleThreadTokenFile::open(
            file.clone(),
            Options {
                default: Some(Token::default()),
                mode: Some(0o600),
            },
        )
        .await
        .unwrap();
        let created = files::permissions(&file).await.unwrap();
        assert_eq!(0o600, created.mode() & 0o777, "create path should be 0o600");

        // update path: a refresh writes a new token and must preserve 0o600
        let token = Token {
            token: "refreshed-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        state_file.write(token).await.unwrap();
        let updated = files::permissions(&file).await.unwrap();
        assert_eq!(0o600, updated.mode() & 0o777, "update path should be 0o600");
    }

    // The default (`mode: None`) leaves permissions unrestricted so non-secret
    // state files (device.json, scanner snapshot) are unaffected.
    #[tokio::test]
    async fn default_mode_does_not_force_0600() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("state.json");

        SingleThreadTokenFile::open(file.clone(), opts_default())
            .await
            .unwrap();
        let perms = files::permissions(&file).await.unwrap();
        assert_ne!(
            0o600,
            perms.mode() & 0o777,
            "default write should not be 0o600"
        );
    }
}

// ========================= MULTI THREADED STATE FILE =========================== //
type ConcurrentTokenFile = ConcurrentStateFile<Token, Updates>;

pub mod spawn_read_only {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        let result = ConcurrentTokenFile::spawn(64, file, Options::default()).await;
        assert!(matches!(result, Err(FileSysErr::PathDoesNotExistErr(_))));
    }

    #[tokio::test]
    async fn exists_invalid_data() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        files::write_string(&file, "invalid-data", WriteOptions::default())
            .await
            .unwrap();

        let result = ConcurrentTokenFile::spawn(64, file, Options::default()).await;
        assert!(matches!(result, Err(FileSysErr::ParseJSONErr(_))));
    }

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        files::write_json(&file, &token, WriteOptions::default())
            .await
            .unwrap();

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file, Options::default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &token);
    }
}

pub mod spawn_with_default_option {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn exists_invalid_data() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        files::write_string(&file, "invalid-data", WriteOptions::default())
            .await
            .unwrap();

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        files::write_json(&file, &token, WriteOptions::default())
            .await
            .unwrap();

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &token);
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        files::write_json(&file, &token, WriteOptions::default())
            .await
            .unwrap();

        let (state_file, handle) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &token);

        // shutdown the file
        state_file.shutdown().await.unwrap();
        handle.await.unwrap();
    }
}

pub mod after_shutdown {
    use super::*;

    #[tokio::test]
    async fn read_fails() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, handle) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();

        state_file.shutdown().await.unwrap();
        handle.await.unwrap();

        assert!(matches!(
            state_file.read().await.unwrap_err(),
            FileSysErr::SendActorMessageErr { .. }
        ));
    }

    #[tokio::test]
    async fn write_fails() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, handle) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();

        state_file.shutdown().await.unwrap();
        handle.await.unwrap();

        assert!(matches!(
            state_file.write(Token::default()).await.unwrap_err(),
            FileSysErr::SendActorMessageErr { .. }
        ));
    }

    #[tokio::test]
    async fn patch_fails() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, handle) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();

        state_file.shutdown().await.unwrap();
        handle.await.unwrap();

        let updates = Updates {
            token: Some("new-token".to_string()),
            expires_at: None,
        };
        assert!(matches!(
            state_file.patch(updates).await.unwrap_err(),
            FileSysErr::SendActorMessageErr { .. }
        ));
    }

    #[tokio::test]
    async fn double_shutdown_fails() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, handle) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();

        state_file.shutdown().await.unwrap();
        handle.await.unwrap();

        assert!(matches!(
            state_file.shutdown().await.unwrap_err(),
            FileSysErr::SendActorMessageErr { .. }
        ));
    }
}

pub mod concurrent_read {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file, opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file.clone(), opts_default())
            .await
            .unwrap();

        // delete the file
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // should still be able to read the file since it's cached in memory
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());
    }
}

pub mod concurrent_write {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());

        // write to the file
        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        state_file.write(token.clone()).await.unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &token);
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());

        // delete the file
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // write to the file
        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        state_file.write(token.clone()).await.unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &token);
    }
}

pub mod concurrent_patch {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());

        // patch the file

        let updates = Updates {
            token: Some("test-token".to_string()),
            expires_at: Some(Utc::now() + Duration::days(1)),
        };
        let expected = Token {
            token: updates.token.clone().unwrap(),
            expires_at: updates.expires_at.unwrap(),
        };
        state_file.patch(updates).await.unwrap();
        assert_eq!(&expected, state_file.read().await.unwrap().as_ref());
    }

    #[tokio::test]
    async fn no_op_skips_write() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let token = Token {
            token: "test-token".to_string(),
            expires_at: Utc::now() + Duration::days(1),
        };
        files::write_json(&file, &token, WriteOptions::default())
            .await
            .unwrap();

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file.clone(), Options::default())
            .await
            .unwrap();

        // delete the backing file so any real write would fail
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // patch with empty updates — merge produces no change, so write is skipped
        state_file.patch(Updates::empty()).await.unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &token);

        // backing file should still not exist (no write occurred)
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file.clone(), opts_default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());

        // delete the file
        files::delete(&file).await.unwrap();
        assert!(!file.exists());

        // patch the file
        let updates = Updates {
            token: Some("test-token".to_string()),
            expires_at: Some(Utc::now() + Duration::days(1)),
        };
        let expected = Token {
            token: updates.token.clone().unwrap(),
            expires_at: updates.expires_at.unwrap(),
        };
        state_file.patch(updates).await.unwrap();
        assert_eq!(&expected, state_file.read().await.unwrap().as_ref());
    }
}

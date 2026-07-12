// internal crates
use miru_agent::authn::token::{Token, Updates};
use miru_agent::filesys::{
    dirs, files,
    state_file::{ConcurrentStateFile, SingleThreadStateFile},
    FileSysErr, Overwrite, PathExt, WriteOptions,
};

// external crates
use chrono::{Duration, Utc};
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

// ========================= SINGLE THREADED STATE FILE =========================== //
type SingleThreadTokenFile = SingleThreadStateFile<Token, Updates>;

pub mod new {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        let result = SingleThreadTokenFile::new(file).await;
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
        let result = SingleThreadTokenFile::new(file).await;
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
        let state_file = SingleThreadTokenFile::new(file).await.unwrap();
        assert_eq!(state_file.read().as_ref(), &token);
    }
}

pub mod new_with_default {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let state_file = SingleThreadTokenFile::new_with_default(file, Token::default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn exists_invalid_data() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        files::write_string(&file, "invalid-data", WriteOptions::default())
            .await
            .unwrap();

        let state_file = SingleThreadTokenFile::new_with_default(file, Token::default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
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
        let state_file = SingleThreadTokenFile::new_with_default(file, Token::default())
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &token);
    }
}

pub mod create {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist_overwrite_false() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let state_file = SingleThreadTokenFile::create(file, &Token::default(), Overwrite::Deny)
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn doesnt_exist_overwrite_true() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let state_file = SingleThreadTokenFile::create(file, &Token::default(), Overwrite::Allow)
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn exists_overwrite_false() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        files::write_string(&file, "invalid-data", WriteOptions::default())
            .await
            .unwrap();

        // should throw an error since already exists
        let result = SingleThreadTokenFile::create(file, &Token::default(), Overwrite::Deny).await;
        assert!(matches!(
            result,
            Err(FileSysErr::InvalidFileOverwriteErr(_))
        ));
    }

    #[tokio::test]
    async fn exists_overwrite_true() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        files::write_string(&file, "invalid-data", WriteOptions::default())
            .await
            .unwrap();

        // should throw an error since already exists
        let state_file = SingleThreadTokenFile::create(file, &Token::default(), Overwrite::Allow)
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
    }
}

pub mod read {
    use super::*;

    #[tokio::test]
    async fn exists() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let state_file = SingleThreadTokenFile::create(file, &Token::default(), Overwrite::Deny)
            .await
            .unwrap();
        assert_eq!(state_file.read().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        // create the file
        let state_file =
            SingleThreadTokenFile::create(file.clone(), &Token::default(), Overwrite::Allow)
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
        let mut state_file =
            SingleThreadTokenFile::create(file, &Token::default(), Overwrite::Deny)
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
        let mut state_file =
            SingleThreadTokenFile::create(file.clone(), &Token::default(), Overwrite::Deny)
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

        let mut state_file =
            SingleThreadTokenFile::create(file, &Token::default(), Overwrite::Deny)
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
        let mut state_file = SingleThreadTokenFile::create(file.clone(), &token, Overwrite::Deny)
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

        let mut state_file =
            SingleThreadTokenFile::create(file.clone(), &Token::default(), Overwrite::Deny)
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

// ========================= MULTI THREADED STATE FILE =========================== //
type ConcurrentTokenFile = ConcurrentStateFile<Token, Updates>;

pub mod spawn {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");
        let result = ConcurrentTokenFile::spawn(64, file).await;
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

        let result = ConcurrentTokenFile::spawn(64, file).await;
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

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file).await.unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &token);
    }
}

pub mod spawn_with_default {
    use super::*;

    #[tokio::test]
    async fn doesnt_exist() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) = ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, _) = ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, _) = ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, handle) =
            ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, handle) =
            ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, handle) =
            ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, handle) =
            ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, handle) =
            ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
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

        let (state_file, _) = ConcurrentTokenFile::spawn_with_default(64, file, Token::default())
            .await
            .unwrap();
        assert_eq!(state_file.read().await.unwrap().as_ref(), &Token::default());
    }

    #[tokio::test]
    async fn file_deleted() {
        let dir = dirs::temp("testing").unwrap();
        let file = dir.file("test-file");

        let (state_file, _) =
            ConcurrentTokenFile::spawn_with_default(64, file.clone(), Token::default())
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

        let (state_file, _) =
            ConcurrentTokenFile::spawn_with_default(64, file.clone(), Token::default())
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

        let (state_file, _) =
            ConcurrentTokenFile::spawn_with_default(64, file.clone(), Token::default())
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

        let (state_file, _) =
            ConcurrentTokenFile::spawn_with_default(64, file.clone(), Token::default())
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

        let (state_file, _) = ConcurrentTokenFile::spawn(64, file.clone()).await.unwrap();

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

        let (state_file, _) =
            ConcurrentTokenFile::spawn_with_default(64, file.clone(), Token::default())
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

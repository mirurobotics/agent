// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_daemon::authn::{AuthnErr, Token, TokenManagerExt};

type RefreshTokenFn = Box<dyn Fn() -> Result<(), AuthnErr> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenManagerCall {
    GetToken,
    RefreshToken,
}

pub struct MockTokenManager {
    pub token: Arc<Mutex<Token>>,
    pub calls: Arc<Mutex<Vec<TokenManagerCall>>>,
    pub refresh_token_fn: Arc<Mutex<RefreshTokenFn>>,
    pub next_get_token_err: Arc<Mutex<Option<AuthnErr>>>,
}

impl MockTokenManager {
    pub fn new(token: Token) -> Self {
        Self {
            token: Arc::new(Mutex::new(token)),
            calls: Arc::new(Mutex::new(Vec::new())),
            refresh_token_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
            next_get_token_err: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_token(&self, token: Token) {
        *self.token.lock().unwrap() = token;
    }

    pub fn get_calls(&self) -> Vec<TokenManagerCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn num_get_token_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == TokenManagerCall::GetToken)
            .count()
    }

    pub fn num_refresh_token_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == TokenManagerCall::RefreshToken)
            .count()
    }

    pub fn set_refresh_token(&self, refresh_token_fn: RefreshTokenFn) {
        *self.refresh_token_fn.lock().unwrap() = refresh_token_fn;
    }

    /// The next `get_token` call returns `err`; later calls return the token.
    pub fn fail_next_get_token(&self, err: AuthnErr) {
        *self.next_get_token_err.lock().unwrap() = Some(err);
    }
}

impl TokenManagerExt for MockTokenManager {
    async fn shutdown(&self) -> Result<(), AuthnErr> {
        Ok(())
    }

    async fn get_token(&self) -> Result<Arc<Token>, AuthnErr> {
        self.calls.lock().unwrap().push(TokenManagerCall::GetToken);
        if let Some(err) = self.next_get_token_err.lock().unwrap().take() {
            return Err(err);
        }
        Ok(Arc::new(self.token.lock().unwrap().clone()))
    }

    async fn refresh_token(&self) -> Result<(), AuthnErr> {
        self.calls
            .lock()
            .unwrap()
            .push(TokenManagerCall::RefreshToken);
        (*self.refresh_token_fn.lock().unwrap())()
    }
}

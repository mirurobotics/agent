// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::authn::{AuthnErr, Token, TokenManagerExt};

type GetTokenFn = Box<dyn Fn() -> Result<Arc<Token>, AuthnErr> + Send + Sync>;
type RefreshTokenFn = Box<dyn Fn() -> Result<(), AuthnErr> + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenManagerCall {
    GetToken,
    RefreshToken,
}

pub struct MockTokenManager {
    pub token: Arc<Mutex<Token>>,
    pub calls: Arc<Mutex<Vec<TokenManagerCall>>>,
    pub get_token_fn: Arc<Mutex<Option<GetTokenFn>>>,
    pub refresh_token_fn: Arc<Mutex<RefreshTokenFn>>,
}

impl MockTokenManager {
    pub fn new(token: Token) -> Self {
        Self {
            token: Arc::new(Mutex::new(token)),
            calls: Arc::new(Mutex::new(Vec::new())),
            get_token_fn: Arc::new(Mutex::new(None)),
            refresh_token_fn: Arc::new(Mutex::new(Box::new(|| Ok(())))),
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

    pub fn set_get_token(&self, get_token_fn: GetTokenFn) {
        *self.get_token_fn.lock().unwrap() = Some(get_token_fn);
    }

    pub fn set_refresh_token(&self, refresh_token_fn: RefreshTokenFn) {
        *self.refresh_token_fn.lock().unwrap() = refresh_token_fn;
    }
}

impl TokenManagerExt for MockTokenManager {
    async fn shutdown(&self) -> Result<(), AuthnErr> {
        Ok(())
    }

    async fn get_token(&self) -> Result<Arc<Token>, AuthnErr> {
        self.calls.lock().unwrap().push(TokenManagerCall::GetToken);
        if let Some(get_token_fn) = &*self.get_token_fn.lock().unwrap() {
            (get_token_fn)()
        } else {
            Ok(Arc::new(self.token.lock().unwrap().clone()))
        }
    }

    async fn refresh_token(&self) -> Result<(), AuthnErr> {
        self.calls
            .lock()
            .unwrap()
            .push(TokenManagerCall::RefreshToken);
        (*self.refresh_token_fn.lock().unwrap())()
    }
}

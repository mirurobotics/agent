// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::{errors::*, issue::issue_token, token::Token, token_mngr::TokenFile};
use crate::filesys::{file::File, path::PathExt};
use crate::http;

/// Single-threaded token manager state and logic. Owned by the token manager
/// [`Worker`](crate::authn::token_mngr::Worker), which serializes access to it.
pub(crate) struct TokenManagerImpl<HTTPClientT: http::ClientI> {
    http_client: Arc<HTTPClientT>,
    token_file: TokenFile,
    private_key_file: File,
    public_key_file: File,
}

impl<HTTPClientT: http::ClientI> TokenManagerImpl<HTTPClientT> {
    pub(crate) fn new(
        http_client: Arc<HTTPClientT>,
        token_file: TokenFile,
        private_key_file: File,
        public_key_file: File,
    ) -> Result<Self, AuthnErr> {
        token_file.file.assert_exists()?;
        private_key_file.assert_exists()?;
        public_key_file.assert_exists()?;
        Ok(Self {
            http_client,
            token_file,
            private_key_file,
            public_key_file,
        })
    }

    pub(crate) async fn get_token(&self) -> Arc<Token> {
        // get the token
        self.token_file.read()
    }

    pub(crate) async fn refresh_token(&mut self) -> Result<(), AuthnErr> {
        // attempt to issue a new token
        let token = self.issue_token().await?;

        // update the token file
        self.token_file.write(token).await?;

        Ok(())
    }

    async fn issue_token(&self) -> Result<Token, AuthnErr> {
        issue_token(
            self.http_client.as_ref(),
            &self.private_key_file,
            &self.public_key_file,
        )
        .await
    }
}

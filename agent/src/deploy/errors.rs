// standard crates
use std::fmt;

// internal crates
use crate::cache::errors::CacheErr;
use crate::crud::errors::CrudErr;
use crate::deploy::fsm;
use crate::errors::{Code, HTTPCode, MiruError, Trace};
use crate::filesys::errors::FileSysErr;
use crate::models::deployment::Deployment;
use crate::storage::errors::StorageErr;

#[derive(Debug)]
pub struct DeploymentNotDeployableErr {
    pub deployment: Deployment,
    pub next_action: fsm::NextAction,
    pub trace: Box<Trace>,
}

impl MiruError for DeploymentNotDeployableErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DeploymentNotDeployableErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot deploy deployment '{:?}' since it's next action is: {:?}",
            self.deployment.id, self.next_action
        )
    }
}

#[derive(Debug)]
pub struct DeploymentNotRemoveableErr {
    pub deployment: Deployment,
    pub next_action: fsm::NextAction,
    pub trace: Box<Trace>,
}

impl MiruError for DeploymentNotRemoveableErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DeploymentNotRemoveableErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot remove deployment '{:?}' since it's next action is: {:?}",
            self.deployment.id, self.next_action
        )
    }
}

#[derive(Debug)]
pub struct DeploymentNotArchiveableErr {
    pub deployment: Deployment,
    pub next_action: fsm::NextAction,
    pub trace: Box<Trace>,
}

impl MiruError for DeploymentNotArchiveableErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DeploymentNotArchiveableErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot archive deployment '{:?}' since it's next action is: {:?}",
            self.deployment.id, self.next_action
        )
    }
}

#[derive(Debug)]
pub struct ConflictingDeploymentsErr {
    pub deployments: Vec<Deployment>,
    pub trace: Box<Trace>,
}

impl MiruError for ConflictingDeploymentsErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for ConflictingDeploymentsErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the following deployments both desire to be deployed: {:?}",
            self.deployments.iter().map(|d| &d.id).collect::<Vec<_>>()
        )
    }
}

#[derive(Debug)]
pub struct DeployFileSysErr {
    pub source: FileSysErr,
    pub trace: Box<Trace>,
}

impl MiruError for DeployFileSysErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for DeployFileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "file system error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct DeployCacheErr {
    pub source: CacheErr,
    pub trace: Box<Trace>,
}

impl MiruError for DeployCacheErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for DeployCacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cache error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct DeployCrudErr {
    pub source: CrudErr,
    pub trace: Box<Trace>,
}

impl MiruError for DeployCrudErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for DeployCrudErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "crud error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct DeployStorageErr {
    pub source: StorageErr,
    pub trace: Box<Trace>,
}

impl MiruError for DeployStorageErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for DeployStorageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "storage error: {}", self.source)
    }
}

#[derive(Debug)]
pub enum DeployErr {
    ConflictingDeploymentsErr(Box<ConflictingDeploymentsErr>),
    DeploymentNotDeployableErr(Box<DeploymentNotDeployableErr>),
    DeploymentNotRemoveableErr(Box<DeploymentNotRemoveableErr>),
    DeploymentNotArchiveableErr(Box<DeploymentNotArchiveableErr>),

    CacheErr(Box<DeployCacheErr>),
    CrudErr(Box<DeployCrudErr>),
    FileSysErr(Box<DeployFileSysErr>),
    StorageErr(Box<DeployStorageErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            DeployErr::ConflictingDeploymentsErr(e) => e.$method($($arg)?),
            DeployErr::DeploymentNotDeployableErr(e) => e.$method($($arg)?),
            DeployErr::DeploymentNotRemoveableErr(e) => e.$method($($arg)?),
            DeployErr::DeploymentNotArchiveableErr(e) => e.$method($($arg)?),

            DeployErr::CacheErr(e) => e.$method($($arg)?),
            DeployErr::CrudErr(e) => e.$method($($arg)?),
            DeployErr::FileSysErr(e) => e.$method($($arg)?),
            DeployErr::StorageErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for DeployErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl MiruError for DeployErr {
    fn code(&self) -> Code {
        forward_error_method!(self, code)
    }

    fn http_status(&self) -> HTTPCode {
        forward_error_method!(self, http_status)
    }

    fn is_network_connection_error(&self) -> bool {
        forward_error_method!(self, is_network_connection_error)
    }

    fn params(&self) -> Option<serde_json::Value> {
        forward_error_method!(self, params)
    }
}

// internal crates
use crate::errors::Trace;
use crate::filesys::FileSysErr;
use crate::models::FileRuleID;

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse date time: {source}")]
pub struct DateTimeParseErr {
    pub source: chrono::ParseError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DateTimeParseErr {}

#[derive(Debug, thiserror::Error)]
#[error("duplicate file rule id in rule set: '{file_rule_id}'")]
pub struct DuplicateFileRuleIDErr {
    pub file_rule_id: FileRuleID,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DuplicateFileRuleIDErr {}

/// Why a file rule set was rejected as a whole.
#[derive(Debug, thiserror::Error)]
pub enum InvalidFileRuleSetErr {
    #[error(transparent)]
    DuplicateFileRuleID(DuplicateFileRuleIDErr),
    #[error(transparent)]
    InvalidGlob(FileSysErr),
}

crate::impl_error!(InvalidFileRuleSetErr {
    DuplicateFileRuleID,
    InvalidGlob,
});

#[derive(Debug, thiserror::Error)]
pub enum ModelsErr {
    #[error(transparent)]
    DateTimeParseErr(DateTimeParseErr),
}

crate::impl_error!(ModelsErr { DateTimeParseErr });

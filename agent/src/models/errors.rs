use crate::errors::Trace;

#[derive(Debug, thiserror::Error)]
#[error("Failed to parse date time: {source}")]
pub struct DateTimeParseErr {
    pub source: chrono::ParseError,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DateTimeParseErr {}

#[derive(Debug, thiserror::Error)]
pub enum ModelsErr {
    #[error(transparent)]
    DateTimeParseErr(DateTimeParseErr),
}

crate::impl_error!(ModelsErr { DateTimeParseErr });

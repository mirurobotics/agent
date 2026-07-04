// internal crates
use crate::cache;
use crate::models;

pub type UploadRules = cache::FileCache<models::UploadRuleID, models::UploadRule>;

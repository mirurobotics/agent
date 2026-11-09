// internal crates
use crate::cache;
use crate::calibrations::ReportMetadata;

// The latest calibrations are split like the config instances: the entries sit in
// one file, keyed by component, role and aspect, while each entry's data, which can
// be large, is its own file, keyed by report id.
pub type LatestCals = cache::FileCache<String, ReportMetadata>;
pub type LatestCalContent = cache::DirCache<String, String>;

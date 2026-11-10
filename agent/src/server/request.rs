// internal crates
use crate::calibrations::Filter;
use crate::models::InstanceFormat;
use crate::server::errors::InvalidRequestErr;
use crate::services::calibration;
use crate::trace;
use device_api::models as device_server;

// external crates
use serde::Deserialize;

/// The role or aspect that the slug filters use to match a missing one, so
/// a report can't use it.
const NULL_SLUG: &str = "null";

/// Checks the request's shape and the reserved [`NULL_SLUG`]; Miru judges
/// the rest.
impl TryFrom<device_server::CreateCalibrationRequest> for calibration::CreateParams {
    type Error = InvalidRequestErr;

    fn try_from(request: device_server::CreateCalibrationRequest) -> Result<Self, Self::Error> {
        for (field, slug) in [("role", &request.role), ("aspect", &request.aspect)] {
            if slug.as_deref() == Some(NULL_SLUG) {
                return Err(InvalidRequestErr {
                    msg: format!("{field} '{NULL_SLUG}' is reserved"),
                    trace: trace!(),
                });
            }
        }
        let content = *request.content;
        let Some(format) = InstanceFormat::from_device_api(content.format) else {
            return Err(InvalidRequestErr {
                msg: "content.format is not a known format".to_string(),
                trace: trace!(),
            });
        };
        Ok(calibration::CreateParams {
            component: request.component,
            role: request.role,
            aspect: request.aspect,
            serial_number: request.serial_number,
            reported_by: request.reported_by,
            format,
            data: content.data,
        })
    }
}

/// The `GET /calibrations/latest` query. Each filter is a comma-separated
/// list, as the spec's `explode: false` sends it.
#[derive(Debug, Default, Deserialize)]
pub struct LatestCalibrationsQuery {
    pub component: Option<String>,
    pub role: Option<String>,
    pub aspect: Option<String>,
}

impl From<LatestCalibrationsQuery> for Filter {
    fn from(query: LatestCalibrationsQuery) -> Self {
        Filter {
            component: split_list(query.component),
            role: split_slugs(query.role),
            aspect: split_slugs(query.aspect),
        }
    }
}

/// The trimmed, non-empty items of a comma-separated list.
pub(crate) fn split_list(list: Option<String>) -> Vec<String> {
    let list = list.unwrap_or_default();
    let items = list.split(',').map(|item| item.trim().to_string());
    items.filter(|item| !item.is_empty()).collect()
}

/// A list of role or aspect slugs, where [`NULL_SLUG`] matches a missing one.
fn split_slugs(list: Option<String>) -> Vec<Option<String>> {
    let slugs = split_list(list).into_iter();
    slugs
        .map(|slug| (slug != NULL_SLUG).then_some(slug))
        .collect()
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    #[test]
    fn split_slugs_maps_null_to_none() {
        let slugs = split_slugs(Some("left, null ,,right".into()));

        assert_eq!(slugs, [Some("left".to_string()), None, Some("right".to_string())]);
    }

    #[test]
    fn split_slugs_matches_null_exactly() {
        let slugs = split_slugs(Some("NULL,nullish".into()));

        assert_eq!(slugs, [Some("NULL".to_string()), Some("nullish".to_string())]);
    }

    #[test]
    fn split_slugs_of_no_list_is_empty() {
        assert!(split_slugs(None).is_empty());
    }
}

/// The trimmed, non-empty items of a comma-separated list.
pub(crate) fn split_list(list: Option<String>) -> Vec<String> {
    let list = list.unwrap_or_default();
    let items = list.split(',').map(|item| item.trim().to_string());
    items.filter(|item| !item.is_empty()).collect()
}

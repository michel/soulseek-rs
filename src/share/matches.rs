/// Check if a string matches a search query
/// Supports space-separated criteria and negative criteria (starting with -)
pub fn matches_query(text: &str, query: &str) -> bool {
    let text_lower = text.to_lowercase();
    query.to_lowercase().split(' ').all(|criterion| {
        if criterion.starts_with('-') {
            !text_lower.contains(&criterion[1..])
        } else {
            text_lower.contains(criterion)
        }
    })
}

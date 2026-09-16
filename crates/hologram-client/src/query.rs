//! The search query, mirrored from the daemon's wire shape.

/// Filters for an object search.
///
/// Every field is an independent conjunctive filter; `None` does not
/// constrain. Results ascend by object id, and `limit` defaults to the
/// daemon's own default when left at zero.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectQuery {
    pub kind: Option<String>,
    pub media_type: Option<String>,
    pub filename_contains: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub created_after_millis: Option<u64>,
    pub created_before_millis: Option<u64>,
    pub limit: u32,
    /// Opaque, and valid only for the daemon that issued it.
    pub cursor: Option<String>,
}

impl ObjectQuery {
    /// Query-string pairs, omitting anything unset.
    ///
    /// Unset fields are left off rather than sent empty, so the daemon applies
    /// its own defaults instead of receiving a filter that matches nothing.
    pub(crate) fn to_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = Vec::new();
        let mut text = |key: &'static str, value: &Option<String>| {
            if let Some(value) = value {
                pairs.push((key, value.clone()));
            }
        };
        text("kind", &self.kind);
        text("media_type", &self.media_type);
        text("filename_contains", &self.filename_contains);
        text("cursor", &self.cursor);

        for (key, value) in [
            ("min_size", self.min_size),
            ("max_size", self.max_size),
            ("created_after_millis", self.created_after_millis),
            ("created_before_millis", self.created_before_millis),
        ] {
            if let Some(value) = value {
                pairs.push((key, value.to_string()));
            }
        }
        if self.limit != 0 {
            pairs.push(("limit", self.limit.to_string()));
        }
        pairs
    }
}

#[cfg(test)]
mod tests {
    use super::ObjectQuery;

    #[test]
    fn an_empty_query_sends_nothing() {
        // Sending empty filters would ask the daemon to match the empty string
        // rather than apply its defaults.
        assert!(ObjectQuery::default().to_pairs().is_empty());
    }

    #[test]
    fn set_filters_are_sent_and_unset_ones_are_omitted() {
        let query = ObjectQuery {
            kind: Some("file".to_owned()),
            min_size: Some(10),
            limit: 25,
            ..ObjectQuery::default()
        };
        let pairs = query.to_pairs();

        assert!(pairs.contains(&("kind", "file".to_owned())));
        assert!(pairs.contains(&("min_size", "10".to_owned())));
        assert!(pairs.contains(&("limit", "25".to_owned())));
        assert!(
            !pairs.iter().any(|(key, _)| *key == "max_size"),
            "an unset filter must not be sent: {pairs:?}"
        );
    }
}

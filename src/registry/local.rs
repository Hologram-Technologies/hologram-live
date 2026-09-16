//! The built-in provider. Content lives in the local `ObjectStore`, so this
//! path needs no network and no external service.

use super::RegistryProvider;
use crate::error::Result;
use crate::protocol::{ObjectContent, ObjectMetadata, ObjectPage, ObjectQuery};
use crate::store::ObjectStore;
use std::sync::Arc;

pub struct LocalRegistryProvider {
    store: Arc<ObjectStore>,
}

impl LocalRegistryProvider {
    pub fn new(store: Arc<ObjectStore>) -> Self {
        Self { store }
    }
}

impl RegistryProvider for LocalRegistryProvider {
    fn list_objects(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>> {
        self.store.list(kind)
    }

    fn put_object(
        &self,
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata> {
        self.store.put(kind, media_type, filename, bytes)
    }

    fn get_object(&self, id: &str) -> Result<ObjectContent> {
        Ok(ObjectContent {
            metadata: self.store.metadata(id)?,
            bytes: self.store.get(id)?,
        })
    }

    fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata> {
        self.store.rename_file(id, filename)
    }

    fn search(&self, query: &ObjectQuery) -> Result<ObjectPage> {
        // The store scan is the only enumeration available locally. It stays a
        // scan deliberately: an index would be a new dependency for a need that
        // has not been demonstrated. The seam is here, so an index can replace
        // this without touching callers.
        let mut all = self.store.list(None)?;
        all.sort_unstable_by(|left, right| left.id.cmp(&right.id));

        let limit = query.effective_limit();
        let after = query.cursor.as_deref();
        let mut objects = Vec::with_capacity(limit.min(all.len()));
        let mut exhausted = true;

        for metadata in all {
            if after.is_some_and(|cursor| metadata.id.as_str() <= cursor) {
                continue;
            }
            if !query.matches(&metadata) {
                continue;
            }
            if objects.len() == limit {
                // A further match exists, so the caller can page again.
                exhausted = false;
                break;
            }
            objects.push(metadata);
        }

        let next_cursor = if exhausted {
            None
        } else {
            objects.last().map(|object| object.id.clone())
        };

        Ok(ObjectPage {
            objects,
            next_cursor,
            // The local scan reads everything, so it never stops early.
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ObjectQuery;

    fn provider(label: &str) -> (LocalRegistryProvider, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hologram-registry-{label}-{}",
            crate::util::now_millis()
        ));
        let store = Arc::new(ObjectStore::open(&root).expect("open"));
        (LocalRegistryProvider::new(store), root)
    }

    #[test]
    fn search_filters_on_kind_and_orders_by_id() {
        let (registry, root) = provider("filter");
        registry
            .put_object(
                "file".into(),
                "text/plain".into(),
                Some("a.txt".into()),
                b"a",
            )
            .expect("put a");
        registry
            .put_object(
                "file".into(),
                "text/plain".into(),
                Some("b.txt".into()),
                b"b",
            )
            .expect("put b");
        registry
            .put_object("holo".into(), "application/octet-stream".into(), None, b"c")
            .expect("put c");

        let page = registry
            .search(&ObjectQuery {
                kind: Some("file".to_owned()),
                ..ObjectQuery::default()
            })
            .expect("search");

        assert_eq!(page.objects.len(), 2, "only file-kind objects match");
        assert!(!page.truncated);
        let ids: Vec<&str> = page.objects.iter().map(|o| o.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "results are ascending by id");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_paginates_with_an_opaque_cursor_and_no_overlap() {
        let (registry, root) = provider("paginate");
        for index in 0..5u8 {
            registry
                .put_object(
                    "file".into(),
                    "text/plain".into(),
                    Some(format!("f{index}.txt")),
                    &[index],
                )
                .expect("put");
        }

        let first = registry
            .search(&ObjectQuery {
                limit: 2,
                ..ObjectQuery::default()
            })
            .expect("first page");
        assert_eq!(first.objects.len(), 2);
        let cursor = first.next_cursor.clone().expect("more results remain");

        let second = registry
            .search(&ObjectQuery {
                limit: 2,
                cursor: Some(cursor),
                ..ObjectQuery::default()
            })
            .expect("second page");
        assert_eq!(second.objects.len(), 2);

        let first_ids: Vec<&String> = first.objects.iter().map(|o| &o.id).collect();
        for object in &second.objects {
            assert!(
                !first_ids.contains(&&object.id),
                "pages must not overlap: {} repeated",
                object.id
            );
        }
        assert!(
            second.objects[0].id > first.objects[1].id,
            "the second page continues strictly after the first"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_filters_on_filename_substring_and_size() {
        let (registry, root) = provider("substring");
        registry
            .put_object(
                "file".into(),
                "text/plain".into(),
                Some("notes-2026.txt".into()),
                b"hello world",
            )
            .expect("put long");
        registry
            .put_object(
                "file".into(),
                "text/plain".into(),
                Some("other.txt".into()),
                b"x",
            )
            .expect("put short");

        let by_name = registry
            .search(&ObjectQuery {
                filename_contains: Some("notes".to_owned()),
                ..ObjectQuery::default()
            })
            .expect("search by name");
        assert_eq!(by_name.objects.len(), 1);
        assert_eq!(
            by_name.objects[0].filename.as_deref(),
            Some("notes-2026.txt")
        );

        let by_size = registry
            .search(&ObjectQuery {
                min_size: Some(5),
                ..ObjectQuery::default()
            })
            .expect("search by size");
        assert_eq!(by_size.objects.len(), 1);
        assert_eq!(by_size.objects[0].size, 11);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_full_final_page_reports_no_further_cursor() {
        let (registry, root) = provider("exact");
        for index in 0..2u8 {
            registry
                .put_object("file".into(), "text/plain".into(), None, &[index])
                .expect("put");
        }

        let page = registry
            .search(&ObjectQuery {
                limit: 2,
                ..ObjectQuery::default()
            })
            .expect("search");

        assert_eq!(page.objects.len(), 2);
        assert!(
            page.next_cursor.is_none(),
            "a page that exhausts the result set must not advertise more"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

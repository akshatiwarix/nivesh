//! The UI parses index names synchronously, so `ui/commands/registry.ts`
//! carries a copy of the index catalogue in `nivesh-core`. This fails when the
//! two drift.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use nivesh_core::INDEX_CATALOGUE;

#[test]
fn ui_registry_lists_every_catalogued_index_with_its_aliases_and_title() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/commands/registry.ts");
    let ts = fs::read_to_string(path).unwrap();
    // Whitespace-insensitive: biome may wrap the object literals.
    let ts: String = ts.split_whitespace().collect();

    for info in &INDEX_CATALOGUE {
        assert!(
            ts.contains(&format!("id:\"{}\"", info.code)),
            "registry.ts has no index with id {}",
            info.code
        );
        assert!(
            ts.contains(&format!("title:\"{}\"", info.title.replace(' ', "")))
                || ts.contains(&format!("title:\"{}\"", info.title)),
            "registry.ts has no title {:?}",
            info.title
        );
        for alias in info.aliases {
            assert!(
                ts.contains(&format!("\"{alias}\"")),
                "registry.ts is missing alias {alias} of {}",
                info.code
            );
        }
    }
}

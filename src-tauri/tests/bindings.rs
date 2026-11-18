//! Fails when `ui/ipc/bindings` is out of date with the Rust IPC types.

// This whole crate is test code; clippy.toml only exempts #[test] fns themselves.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn read_ts_files(dir: &Path) -> BTreeMap<String, String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return BTreeMap::new();
    };
    entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ts"))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // Tolerate CRLF checkouts (e.g. git autocrlf on Windows).
            let text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
            (name, text)
        })
        .collect()
}

#[test]
fn committed_bindings_are_fresh() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ipc-bindings");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    nivesh_app::ipc::bindings::export(&scratch).unwrap();

    let expected = read_ts_files(&scratch);
    let committed = read_ts_files(&nivesh_app::ipc::bindings::committed_dir());
    assert!(!expected.is_empty(), "no bindings were generated");

    let mut problems = Vec::new();
    for (name, text) in &expected {
        match committed.get(name) {
            None => problems.push(format!("missing {name}")),
            Some(found) if found != text => problems.push(format!("outdated {name}")),
            Some(_) => {}
        }
    }
    for name in committed
        .keys()
        .filter(|name| !expected.contains_key(*name))
    {
        problems.push(format!("stale {name} (no matching Rust type)"));
    }
    assert!(
        problems.is_empty(),
        "ui/ipc/bindings is out of date: {}.\nRun `pnpm gen:bindings` and commit the result.",
        problems.join(", ")
    );
}

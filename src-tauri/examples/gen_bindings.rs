//! Regenerates `ui/ipc/bindings`. Run with `pnpm gen:bindings`.

use std::fs;

fn main() -> anyhow::Result<()> {
    let dir = nivesh_app::ipc::bindings::committed_dir();
    fs::create_dir_all(&dir)?;
    // Start from empty so bindings for removed or renamed types don't linger.
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "ts") {
            fs::remove_file(&path)?;
        }
    }
    nivesh_app::ipc::bindings::export(&dir)?;
    let shown = dir.canonicalize().unwrap_or(dir);
    println!("wrote IPC bindings to {}", shown.display());
    Ok(())
}

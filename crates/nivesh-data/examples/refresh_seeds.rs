//! Regenerates the data files bundled with `nivesh-data` from the public
//! sources, so the app has something to show on a first, offline run.
//!
//! ```text
//! cargo run -p nivesh-data --example refresh_seeds -- constituents
//! cargo run -p nivesh-data --example refresh_seeds -- bse
//! cargo run -p nivesh-data --example refresh_seeds -- all   # also `pnpm refresh:data`
//! ```
//!
//! Needs the network and is run by hand, never in CI. Review the diff, then
//! commit the regenerated files under `crates/nivesh-data/data/`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nivesh_core::Exchange;
use nivesh_data::bse::BseListProvider;
use nivesh_data::constituents;
use nivesh_data::http::{HostLimits, HttpClient, HttpConfig};
use nivesh_data::{DataProvider as _, write_list_csv};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

async fn run(what: &str) -> Result<(), String> {
    let http = HttpClient::new(HttpConfig::default(), Arc::new(HostLimits::defaults()))
        .map_err(|e| e.to_string())?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let now_ms = i64::try_from(now_ms).map_err(|e| e.to_string())?;

    if matches!(what, "constituents" | "all") {
        let bases: Vec<String> = constituents::DEFAULT_BASES
            .iter()
            .map(|b| (*b).to_owned())
            .collect();
        let set = constituents::fetch_set(&http, &bases, &constituents::ist_date(now_ms))
            .await
            .map_err(|e| format!("constituents: {e}"))?;
        let path = data_dir().join("index_constituents.json");
        let json = serde_json::to_string_pretty(&set).map_err(|e| e.to_string())?;
        std::fs::write(&path, json + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
        println!(
            "constituents: {} lists as of {} -> {}",
            set.lists.len(),
            set.as_of,
            path.display()
        );
    }
    if matches!(what, "bse" | "all") {
        let list = BseListProvider::new(http.clone())
            .instruments(Exchange::Bse)
            .await
            .map_err(|e| format!("bse: {e}"))?;
        let path = data_dir().join("bse_equity_seed.csv");
        std::fs::write(&path, write_list_csv(&list))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        println!("bse: {} equities -> {}", list.len(), path.display());
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let what = std::env::args().nth(1).unwrap_or_default();
    if !matches!(what.as_str(), "constituents" | "bse" | "all") {
        eprintln!("usage: refresh_seeds <constituents|bse|all>");
        return ExitCode::from(2);
    }
    match run(&what).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("refresh failed: {err}");
            ExitCode::FAILURE
        }
    }
}

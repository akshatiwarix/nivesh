//! A start-up failure the user must see: a native alert with the reason,
//! since the window never opened. Best effort: when no alert can be shown
//! the reason is still in the log and on stderr.

/// What to tell the user, with a hint for failures they can fix.
pub fn explain(message: &str) -> String {
    let hint = if message.contains("is newer than this build supports") {
        "\n\nThis copy of Nivesh is older than the one that last used your data. \
         Install the latest Nivesh, or open the newer copy you already have."
    } else {
        ""
    };
    format!("{message}{hint}")
}

pub fn show(message: &str) {
    let text = explain(message);
    #[cfg(target_os = "macos")]
    {
        // The text goes in as an argument, never into the script itself.
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display alert \"Nivesh could not start\" message (item 1 of argv) as critical",
                "-e",
                "end run",
                "--",
                &text,
            ])
            .status();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("zenity")
            .args([
                "--error",
                "--title=Nivesh could not start",
                "--no-markup",
                "--text",
                &text,
            ])
            .status();
    }
}

/// Keeps `explain`'s match in step with the store's wording.
#[cfg(test)]
mod tests {
    use super::*;
    use nivesh_store::StoreError;

    #[test]
    fn a_newer_database_gets_a_hint() {
        let err = StoreError::SchemaTooNew {
            found: 10,
            supported: 8,
        };
        assert!(explain(&err.to_string()).contains("Install the latest Nivesh"));
        assert_eq!(explain("disk full"), "disk full");
    }
}

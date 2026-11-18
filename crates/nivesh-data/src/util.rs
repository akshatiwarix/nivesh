//! Small helpers shared by providers.

use crate::DataError;

/// Percent-encodes a URL path segment or query value. Keeps unreserved
/// characters (`A-Z a-z 0-9 - _ . ~`) and encodes everything else.
pub(crate) fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Collects per-item results: the successes if there are any, otherwise the
/// most informative failure (anything but `NotFound` wins over `NotFound`).
pub(crate) fn settle<T>(results: Vec<Result<T, DataError>>) -> Result<Vec<T>, DataError> {
    let mut ok = Vec::new();
    let mut err: Option<DataError> = None;
    for r in results {
        match r {
            Ok(v) => ok.push(v),
            Err(DataError::NotFound) => {
                err.get_or_insert(DataError::NotFound);
            }
            Err(e) => {
                if matches!(err, None | Some(DataError::NotFound)) {
                    err = Some(e);
                }
            }
        }
    }
    if ok.is_empty() {
        Err(err.unwrap_or(DataError::NotFound))
    } else {
        Ok(ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_reserved_characters() {
        assert_eq!(percent_encode("NIFTY 50"), "NIFTY%2050");
        assert_eq!(percent_encode("M&M.NS"), "M%26M.NS");
        assert_eq!(percent_encode("^NSEI"), "%5ENSEI");
        assert_eq!(percent_encode("BAJAJ-AUTO"), "BAJAJ-AUTO");
    }

    #[test]
    fn settle_prefers_successes_then_real_errors_over_not_found() {
        assert_eq!(settle(vec![Ok(1), Err(DataError::Timeout)]), Ok(vec![1]));
        assert_eq!(
            settle::<i32>(vec![Err(DataError::NotFound), Err(DataError::Timeout)]),
            Err(DataError::Timeout)
        );
        assert_eq!(
            settle::<i32>(vec![Err(DataError::Timeout), Err(DataError::NotFound)]),
            Err(DataError::Timeout)
        );
        assert_eq!(
            settle::<i32>(vec![Err(DataError::NotFound)]),
            Err(DataError::NotFound)
        );
        assert_eq!(settle::<i32>(vec![]), Err(DataError::NotFound));
    }
}

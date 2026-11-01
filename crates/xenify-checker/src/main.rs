//! `xenify-checker`: the shared background checker (see the library docs).
//! Apps start it themselves; it exits once no app has used it for a while.
//!
//!     xenify-checker [--grace SECONDS]
//!
//! `XENIFY_CHECKER_GRACE` (seconds) also sets the grace, for tests.

use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let mut grace = std::env::var("XENIFY_CHECKER_GRACE")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(xenify_checker::GRACE);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match (a.as_str(), args.next().and_then(|s| s.parse::<u64>().ok())) {
            ("--grace", Some(s)) => grace = Duration::from_secs(s),
            _ => {
                eprintln!("usage: xenify-checker [--grace SECONDS]");
                return ExitCode::FAILURE;
            }
        }
    }
    let path = match xenify_checker::socket_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("xenify-checker: {e}");
            return ExitCode::FAILURE;
        }
    };
    match xenify_checker::serve(&path, grace) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xenify-checker: {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

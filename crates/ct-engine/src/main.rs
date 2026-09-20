//! Executable entry point for the `ct-engine` JSONL sidecar.
//!
//! The process reads one bounded request per stdin line, writes one response per
//! stdout line, and shuts the domain engine down when its desktop parent closes
//! the pipe. Business rules remain in the library so they are testable.

use ct_engine::{timestamp, Engine, Request, Response, PROTOCOL_VERSION};
use serde_json::json;
use std::io::{self, BufRead, Write};

fn main() -> io::Result<()> {
    // Explicit developer switch only; the default process is production locked.
    let preview = std::env::args().skip(1).any(|arg| arg == "--preview");
    let mut engine = Engine::new(preview);
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) if line.len() <= 65536 => engine.handle(request),
            _ => Response {
                protocol_version: PROTOCOL_VERSION,
                request_id: String::new(),
                command: "invalid".into(),
                payload: json!({"message":"Expected a versioned JSONL envelope, at most 64 KiB"}),
                timestamp: timestamp(),
                sequence: 0,
                error_code: Some("INVALID_ENVELOPE".into()),
            },
        };
        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(&mut stdout)?;
        stdout.flush()?;
    }
    // stdin EOF means the desktop shell is gone (closed, crashed or killed).
    // Shut down deterministically: stop any active scan and force the Moxtek
    // OFF before this process exits, so a terminated UI can never leave the
    // tube emitting.
    engine.shutdown();
    Ok(())
}

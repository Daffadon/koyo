use std::fs::File;
use std::io::Read;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

static EXECUTABLE_CHECKSUM: OnceLock<String> = OnceLock::new();

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// SHA-256 of the running executable. The value identifies the exact binary,
/// not just the source revision.
pub(crate) fn executable_checksum() -> String {
    EXECUTABLE_CHECKSUM
        .get_or_init(|| match executable_checksum_inner() {
            Ok(checksum) => checksum,
            Err(error) => format!("Unavailable: {error}"),
        })
        .clone()
}

fn executable_checksum_inner() -> Result<String, String> {
    let path = std::env::current_exe()
        .map_err(|error| format!("could not locate the executable: {error}"))?;
    let mut file =
        File::open(&path).map_err(|error| format!("could not read the executable: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("could not read the executable: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

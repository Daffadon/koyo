use std::fs::File;
use std::io::Read;
use std::sync::{Once, OnceLock};

use sha2::{Digest, Sha256};

static EXECUTABLE_CHECKSUM: OnceLock<String> = OnceLock::new();
static CRYPTO_PROVIDER: Once = Once::new();

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Prefer the hybrid X25519MLKEM768 key exchange whenever a TLS peer supports
/// it. This is idempotent because tests create clients without going through
/// `main`.
pub(crate) fn ensure_crypto_provider() {
    CRYPTO_PROVIDER.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

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

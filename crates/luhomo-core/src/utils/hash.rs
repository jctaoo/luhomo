use std::{io, path::Path};

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

/// Calculates the SHA-256 hash of a file.
///
/// The returned value is a lowercase hexadecimal string.
pub async fn file_sha256(path: impl AsRef<Path>) -> io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let bytes_read = file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);

    for &byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::file_sha256;

    #[tokio::test]
    async fn hashes_file_contents() {
        let path = std::env::temp_dir().join(format!(
            "luhomo-core-file-sha256-{}",
            uuid::Uuid::new_v4()
        ));
        tokio::fs::write(&path, b"hello world").await.unwrap();

        let hash = file_sha256(&path).await.unwrap();

        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn returns_error_for_missing_file() {
        let result = file_sha256(std::env::temp_dir().join("luhomo-core-file-does-not-exist"))
            .await;

        assert!(result.is_err());
    }
}

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use md5::Md5;
use sha1::Sha1;
use sha2::digest::Digest;
use sha2::{Sha256, Sha512};

use crate::model::{ChecksumAlgorithm, ChecksumSpec, DownloadIntegrity, IntegrityState};

const HASH_BUFFER_BYTES: usize = 1024 * 1024;
pub(super) const INTEGRITY_PENDING_MESSAGE: &str = "Checksum validation will run after completion.";
pub(super) const INTEGRITY_VERIFYING_MESSAGE: &str = "Verifying completed file checksum.";
const INTEGRITY_VERIFIED_MESSAGE: &str = "Checksum verified successfully.";
const INTEGRITY_MISMATCH_MESSAGE: &str = "Checksum mismatch detected.";
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

fn clear_integrity_observation(integrity: &mut DownloadIntegrity) {
    integrity.actual = None;
    integrity.checked_at = None;
}

pub(super) fn normalize_checksum_spec(spec: ChecksumSpec) -> Result<ChecksumSpec, String> {
    let normalized = spec.value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err("Checksum cannot be empty.".to_string());
    }
    if !normalized.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err("Checksum must contain only hexadecimal characters.".to_string());
    }

    let expected_len = expected_hash_len(&spec.algorithm);
    if normalized.len() != expected_len {
        return Err(format!(
            "{} checksums must be exactly {} hexadecimal characters.",
            algorithm_label(&spec.algorithm),
            expected_len
        ));
    }

    Ok(ChecksumSpec {
        algorithm: spec.algorithm,
        value: normalized,
    })
}

pub(super) fn reset_integrity_for_expected(
    integrity: &mut DownloadIntegrity,
    expected: Option<ChecksumSpec>,
) {
    integrity.expected = expected;
    clear_integrity_observation(integrity);
    if integrity.expected.is_some() {
        integrity.state = IntegrityState::Pending;
        integrity.message = Some(INTEGRITY_PENDING_MESSAGE.to_string());
    } else {
        integrity.state = IntegrityState::None;
        integrity.message = None;
    }
}

pub(super) fn mark_integrity_verifying(integrity: &mut DownloadIntegrity) {
    clear_integrity_observation(integrity);
    integrity.state = IntegrityState::Verifying;
    integrity.message = Some(INTEGRITY_VERIFYING_MESSAGE.to_string());
}

pub(super) fn apply_integrity_result(
    integrity: &mut DownloadIntegrity,
    actual: String,
    matched: bool,
    checked_at: i64,
) {
    integrity.actual = Some(actual);
    integrity.checked_at = Some(checked_at);
    integrity.state = if matched {
        IntegrityState::Verified
    } else {
        IntegrityState::Mismatch
    };
    integrity.message = Some(if matched {
        INTEGRITY_VERIFIED_MESSAGE.to_string()
    } else {
        INTEGRITY_MISMATCH_MESSAGE.to_string()
    });
}

pub(super) fn mark_integrity_failure(integrity: &mut DownloadIntegrity, error: &str) {
    clear_integrity_observation(integrity);
    integrity.state = IntegrityState::Pending;
    integrity.message = Some(format!("Checksum verification failed: {error}"));
}

pub(super) async fn compute_checksum(path: PathBuf, spec: ChecksumSpec) -> Result<String, String> {
    tokio::task::spawn_blocking(move || compute_checksum_sync(path, spec))
        .await
        .map_err(|error| format!("Checksum worker failed: {error}"))?
}

fn compute_checksum_sync(path: PathBuf, spec: ChecksumSpec) -> Result<String, String> {
    let mut file = File::open(&path).map_err(|error| {
        format!(
            "Failed opening '{}' for checksum verification: {error}",
            path.display()
        )
    })?;

    match spec.algorithm {
        ChecksumAlgorithm::Md5 => hash_file::<Md5>(&mut file),
        ChecksumAlgorithm::Sha1 => hash_file::<Sha1>(&mut file),
        ChecksumAlgorithm::Sha256 => hash_file::<Sha256>(&mut file),
        ChecksumAlgorithm::Sha512 => hash_file::<Sha512>(&mut file),
    }
}

fn hash_file<D: Digest + Default>(file: &mut File) -> Result<String, String> {
    let mut hasher = D::default();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            format!("Failed reading file during checksum verification: {error}")
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(encode_lower_hex(digest.as_ref()))
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        encoded.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        encoded.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn expected_hash_len(algorithm: &ChecksumAlgorithm) -> usize {
    match algorithm {
        ChecksumAlgorithm::Md5 => 32,
        ChecksumAlgorithm::Sha1 => 40,
        ChecksumAlgorithm::Sha256 => 64,
        ChecksumAlgorithm::Sha512 => 128,
    }
}

fn algorithm_label(algorithm: &ChecksumAlgorithm) -> &'static str {
    match algorithm {
        ChecksumAlgorithm::Md5 => "MD5",
        ChecksumAlgorithm::Sha1 => "SHA-1",
        ChecksumAlgorithm::Sha256 => "SHA-256",
        ChecksumAlgorithm::Sha512 => "SHA-512",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::model::{ChecksumAlgorithm, ChecksumSpec, DownloadIntegrity, IntegrityState};

    use super::{
        apply_integrity_result, compute_checksum_sync, mark_integrity_verifying,
        normalize_checksum_spec, reset_integrity_for_expected, INTEGRITY_PENDING_MESSAGE,
    };

    #[test]
    fn normalizes_checksum_specs() {
        let spec = normalize_checksum_spec(ChecksumSpec {
            algorithm: ChecksumAlgorithm::Sha256,
            value: " A0B1C2D3E4F5A6B7C8D9E0F1A2B3C4D5E6F70123456789ABCDEF001122334455 ".to_string(),
        })
        .unwrap_or_else(|error| panic!("unexpected checksum normalization failure: {error}"));

        assert_eq!(
            spec.value,
            "a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f70123456789abcdef001122334455"
        );
    }

    #[test]
    fn rejects_invalid_checksum_lengths() {
        let error = normalize_checksum_spec(ChecksumSpec {
            algorithm: ChecksumAlgorithm::Md5,
            value: "abcd".to_string(),
        })
        .expect_err("short MD5 hashes should be rejected");

        assert!(error.contains("32 hexadecimal characters"));
    }

    #[test]
    fn computes_sha256_hashes_from_files() {
        let path =
            std::env::temp_dir().join(format!("vdm-integrity-test-{}.bin", std::process::id()));
        fs::write(&path, b"vdm-integrity-test")
            .unwrap_or_else(|error| panic!("failed writing integrity test file: {error}"));

        let digest = compute_checksum_sync(
            path.clone(),
            ChecksumSpec {
                algorithm: ChecksumAlgorithm::Sha256,
                value: "0".repeat(64),
            },
        )
        .unwrap_or_else(|error| panic!("unexpected checksum failure: {error}"));

        fs::remove_file(&path)
            .unwrap_or_else(|error| panic!("failed removing integrity test file: {error}"));
        assert_eq!(
            digest,
            "21107cdd08f5b9c083da2c9f9c2c24c0479a15c5ee2840ade421f74757c78fbb"
        );
    }

    #[test]
    fn reset_integrity_marks_expected_checksums_pending() {
        let mut integrity = DownloadIntegrity::default();

        reset_integrity_for_expected(
            &mut integrity,
            Some(ChecksumSpec {
                algorithm: ChecksumAlgorithm::Sha1,
                value: "0".repeat(40),
            }),
        );
        assert_eq!(integrity.state, IntegrityState::Pending);

        mark_integrity_verifying(&mut integrity);
        assert_eq!(integrity.state, IntegrityState::Verifying);
    }

    #[test]
    fn mismatch_results_keep_actual_digest_for_user_recovery() {
        let mut integrity = DownloadIntegrity::default();
        let actual = "f".repeat(64);
        reset_integrity_for_expected(
            &mut integrity,
            Some(ChecksumSpec {
                algorithm: ChecksumAlgorithm::Sha256,
                value: "0".repeat(64),
            }),
        );

        apply_integrity_result(&mut integrity, actual.clone(), false, 1_710_000_000_000);

        assert_eq!(integrity.state, IntegrityState::Mismatch);
        assert_eq!(integrity.actual.as_deref(), Some(actual.as_str()));
        assert_eq!(integrity.checked_at, Some(1_710_000_000_000));
        assert_eq!(
            integrity.message.as_deref(),
            Some("Checksum mismatch detected.")
        );
    }

    #[test]
    fn resetting_expected_checksum_clears_previous_mismatch_state() {
        let mut integrity = DownloadIntegrity::default();
        let next_expected = "1".repeat(40);
        reset_integrity_for_expected(
            &mut integrity,
            Some(ChecksumSpec {
                algorithm: ChecksumAlgorithm::Sha1,
                value: "0".repeat(40),
            }),
        );
        apply_integrity_result(&mut integrity, "f".repeat(40), false, 42);

        reset_integrity_for_expected(
            &mut integrity,
            Some(ChecksumSpec {
                algorithm: ChecksumAlgorithm::Sha1,
                value: next_expected.clone(),
            }),
        );

        assert_eq!(integrity.state, IntegrityState::Pending);
        assert_eq!(integrity.actual, None);
        assert_eq!(integrity.checked_at, None);
        assert_eq!(
            integrity.message.as_deref(),
            Some(INTEGRITY_PENDING_MESSAGE)
        );
        assert_eq!(
            integrity
                .expected
                .as_ref()
                .map(|value| value.value.as_str()),
            Some(next_expected.as_str())
        );
    }
}

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

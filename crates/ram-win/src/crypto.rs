//! Random bytes and SHA-256 from the Windows CNG provider (for OAuth PKCE).

use windows::Win32::Security::Cryptography::{
    BCryptGenRandom, BCryptHash, BCRYPT_SHA256_ALG_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};

/// Cryptographically secure random bytes, or `None` if the system RNG fails.
pub fn try_random_bytes(n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    if n == 0 {
        return Some(buf);
    }
    let status = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    status.is_ok().then_some(buf)
}

/// Cryptographically secure random bytes.
///
/// # Panics
/// Panics if the system RNG reports failure, because continuing with
/// predictable bytes would be worse. Use [`try_random_bytes`] to handle it.
pub fn random_bytes(n: usize) -> Vec<u8> {
    try_random_bytes(n).expect("the Windows system RNG failed")
}

/// SHA-256 of `data`, or `None` if the CNG provider fails.
pub fn try_sha256(data: &[u8]) -> Option<[u8; 32]> {
    let mut digest = [0u8; 32];
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut digest) };
    status.is_ok().then_some(digest)
}

/// SHA-256 of `data`.
///
/// # Panics
/// Panics if the CNG SHA-256 provider reports failure. Use [`try_sha256`]
/// to handle it.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    try_sha256(data).expect("the Windows SHA-256 provider failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn random_bytes_have_requested_length_and_vary() {
        assert!(random_bytes(0).is_empty());
        let a = random_bytes(32);
        let b = random_bytes(32);
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert_ne!(a, vec![0u8; 32]);
    }
}

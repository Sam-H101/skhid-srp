//! Passwords, kept only as salted argon2id hashes in the PHC string form
//! (`$argon2id$v=19$m=...,t=...,p=...$<salt>$<hash>`). The password itself
//! is never stored or logged. Hashing and checking take tens of
//! milliseconds: call them with no lock held.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use password_hash::rand_core::OsRng;

/// A new hash of `password`, with a fresh random salt. `None` only if the
/// hasher fails, which its default parameters never make it do.
pub fn hash(password: &[u8]) -> Option<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password, &salt)
        .ok()
        .map(|hash| hash.to_string())
}

/// Whether `password` is the one `hash` was made from. The comparison takes
/// the same time however much of it matches. A hash that can't be read
/// matches nothing.
pub fn verify(hash: &str, password: &[u8]) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default().verify_password(password, &parsed).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_is_argon2id_salted_and_never_holds_the_password() {
        let first = hash(b"test-password-1").unwrap();
        assert!(first.starts_with("$argon2id$"));
        assert!(!first.contains("test-password-1"));
        // A fresh salt every time.
        assert_ne!(first, hash(b"test-password-1").unwrap());
        assert!(verify(&first, b"test-password-1"));
        assert!(!verify(&first, b"test-password-2"));
        assert!(!verify(&first, b""));
        assert!(!verify("not a hash", b"test-password-1"));
    }
}

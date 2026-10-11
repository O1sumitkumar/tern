//! The vault passphrase sealed to a short PIN, for unlocking after an idle lock without typing
//! the whole passphrase.
//!
//! The sealed blob is an age file with an scrypt recipient, like the vault. A PIN has little
//! entropy, so it is never a way into the vault file by itself: the app keeps the blob in the
//! login Keychain, and what a PIN alone opens is nothing. Wrong guesses are counted by the app
//! (`vault_pin.rs`), which deletes the blob after too many.

use crate::{ExposeSecret, SecretString, VaultError, Zeroizing};

/// scrypt cost, log2(N): a PIN unlock should feel quick, and the Keychain, not the cost,
/// is what keeps the blob out of reach.
const WORK_FACTOR: u8 = 16;

/// The fewest and most digits a PIN has.
pub const MIN_DIGITS: usize = 4;
pub const MAX_DIGITS: usize = 8;

/// Why `pin` cannot be a PIN, in words for the form; `None` when it can.
pub fn problem(pin: &str) -> Option<&'static str> {
    if !pin.chars().all(|c| c.is_ascii_digit()) {
        Some("A PIN is digits only.")
    } else if !(MIN_DIGITS..=MAX_DIGITS).contains(&pin.len()) {
        Some("A PIN is 4 to 8 digits.")
    } else {
        None
    }
}

/// Seals `passphrase` to `pin`.
///
/// # Errors
/// [`VaultError::Corrupt`] if encryption fails, which would be a bug.
pub fn wrap(passphrase: &SecretString, pin: &str) -> Result<Vec<u8>, VaultError> {
    wrap_with(passphrase, pin, WORK_FACTOR)
}

fn wrap_with(passphrase: &SecretString, pin: &str, work_factor: u8) -> Result<Vec<u8>, VaultError> {
    let mut recipient = age::scrypt::Recipient::new(SecretString::from(pin.to_owned()));
    recipient.set_work_factor(work_factor);
    age::encrypt(&recipient, passphrase.expose_secret().as_bytes())
        .map_err(|e| VaultError::Corrupt(e.to_string()))
}

/// The passphrase `blob` holds, when `pin` is the one it was sealed to.
///
/// # Errors
/// [`VaultError::WrongPassphrase`] for a wrong PIN or an altered blob.
pub fn unwrap(blob: &[u8], pin: &str) -> Result<SecretString, VaultError> {
    let identity = age::scrypt::Identity::new(SecretString::from(pin.to_owned()));
    let plain =
        Zeroizing::new(age::decrypt(&identity, blob).map_err(|_| VaultError::WrongPassphrase)?);
    String::from_utf8(plain.to_vec())
        .map(SecretString::from)
        .map_err(|_| VaultError::Corrupt("the sealed passphrase is not text".into()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[test]
    fn the_pin_gives_back_exactly_the_passphrase_it_sealed() {
        // Not a palindrome, with characters that survive only if nothing is trimmed or lossy.
        let pass = secret(" correct horse: battery\nstaple é ");
        let blob = wrap_with(&pass, "4821", 10).unwrap();
        let back = unwrap(&blob, "4821").unwrap();
        assert_eq!(back.expose_secret(), pass.expose_secret());
        assert!(
            !String::from_utf8_lossy(&blob).contains("correct horse"),
            "the blob is sealed"
        );
    }

    /// The private `wrap_with` is used above for speed; this exercises the public `wrap` at its
    /// real work factor, so the actual PIN-seal path round-trips and an empty/garbage seal fails.
    #[test]
    fn the_public_wrap_round_trips_through_unwrap() {
        let pass = secret("battery horse é\nstaple ");
        let blob = wrap(&pass, "4821").unwrap();
        assert!(!blob.is_empty(), "an empty seal would open to nothing");
        assert_eq!(unwrap(&blob, "4821").unwrap().expose_secret(), pass.expose_secret());
    }

    #[test]
    fn a_wrong_pin_opens_nothing() {
        let blob = wrap_with(&secret("pass"), "4821", 10).unwrap();
        for wrong in ["4822", "1284", "482", "48210", ""] {
            assert!(
                matches!(unwrap(&blob, wrong), Err(VaultError::WrongPassphrase)),
                "{wrong:?} must not open it"
            );
        }
    }

    #[test]
    fn an_altered_blob_is_refused() {
        let mut blob = wrap_with(&secret("pass"), "4821", 10).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(unwrap(&blob, "4821").is_err());
    }

    #[test]
    fn a_pin_is_four_to_eight_digits() {
        assert_eq!(problem("1234"), None);
        assert_eq!(problem("12345678"), None);
        assert!(problem("123").is_some());
        assert!(problem("123456789").is_some());
        assert!(problem("12a4").is_some());
        assert!(problem("12 4").is_some());
        assert!(problem("").is_some());
        // Digits that are not ASCII are not accepted either.
        assert!(problem("١٢٣٤").is_some());
    }
}

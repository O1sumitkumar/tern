//! One encrypted file holding SSH secrets, unlocked with a passphrase.
//!
//! The feature is Tabby's vault (`tabby-core/src/services/vault.service.ts`): passwords per
//! `user@host:port`, passphrases per key file. The crypto is not Tabby's (PBKDF2 + AES-CBC with
//! no MAC, so tampering goes unnoticed): the file is an [age] file with an scrypt passphrase
//! recipient, which is authenticated, so a wrong passphrase and a tampered file are both refused.
//!
//! Plaintext exists only inside this crate, in buffers zeroed on drop; callers get
//! [`SecretString`]s.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

pub mod keys;
pub mod pin;

pub use age::secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
pub use zeroize::Zeroizing;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// What a secret unlocks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Key {
    /// A login password for `user@host:port`.
    Password {
        user: String,
        host: String,
        port: u16,
    },
    /// The passphrase of a private key file, by its path.
    KeyPassphrase { path: String },
    /// A private key kept in the vault, by the name the person gave it. The value is the key
    /// in OpenSSH text form.
    SshKey { name: String },
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("wrong passphrase, or the vault file was modified")]
    WrongPassphrase,
    #[error("vault file is not a tern vault: {0}")]
    Corrupt(String),
    #[error("vault file: {0}")]
    Io(#[from] io::Error),
}

/// On-disk entry. `value` is plaintext only between decryption and conversion, and is zeroed
/// on drop.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct Entry {
    #[zeroize(skip)]
    key: Key,
    value: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Contents {
    version: u32,
    entries: Vec<Entry>,
}

const VERSION: u32 = 1;
/// scrypt cost, log2(N). age's own default targets about a second; 18 is that on current Macs.
const WORK_FACTOR: u8 = 18;

/// A new file only the owner can read (mode 0600; on Windows the file inherits the owner-only
/// access of the user's profile directory it is written under).
fn private_file(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

/// An unlocked vault: secrets in memory, the passphrase kept to re-encrypt on save.
pub struct Vault {
    passphrase: SecretString,
    entries: Vec<(Key, SecretString)>,
    work_factor: u8,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl Vault {
    /// An empty vault that will be encrypted with `passphrase`.
    pub fn new(passphrase: SecretString) -> Self {
        Self {
            passphrase,
            entries: Vec::new(),
            work_factor: WORK_FACTOR,
        }
    }

    /// Decrypts the vault at `path`.
    ///
    /// # Errors
    /// [`VaultError::WrongPassphrase`] when the passphrase does not open it or the file was
    /// altered; [`VaultError::Corrupt`] when it is not an age file or holds unexpected data;
    /// [`VaultError::Io`] when it cannot be read.
    pub fn unlock(path: &Path, passphrase: SecretString) -> Result<Self, VaultError> {
        Self::unlock_bytes(&std::fs::read(path)?, passphrase)
    }

    /// [`Vault::unlock`] for bytes already in memory, such as a vault downloaded by sync.
    ///
    /// # Errors
    /// As [`Vault::unlock`], without the I/O case.
    pub fn unlock_bytes(ciphertext: &[u8], passphrase: SecretString) -> Result<Self, VaultError> {
        let identity = age::scrypt::Identity::new(passphrase.clone());
        let plaintext =
            Zeroizing::new(age::decrypt(&identity, ciphertext).map_err(|e| match e {
                age::DecryptError::DecryptionFailed
                | age::DecryptError::KeyDecryptionFailed
                | age::DecryptError::NoMatchingKeys
                | age::DecryptError::InvalidMac => VaultError::WrongPassphrase,
                other => VaultError::Corrupt(other.to_string()),
            })?);
        let mut contents: Contents =
            serde_json::from_slice(&plaintext).map_err(|e| VaultError::Corrupt(e.to_string()))?;
        if contents.version != VERSION {
            return Err(VaultError::Corrupt(format!(
                "version {} is not {VERSION}",
                contents.version
            )));
        }
        let entries = contents
            .entries
            .iter_mut()
            .map(|entry| {
                let value = SecretString::from(std::mem::take(&mut entry.value));
                (entry.key.clone(), value)
            })
            .collect();
        Ok(Self {
            passphrase,
            entries,
            work_factor: WORK_FACTOR,
        })
    }

    /// Encrypts and writes the vault to `path` via a temp file and rename, so an interrupted
    /// save leaves the previous vault intact.
    ///
    /// # Errors
    /// [`VaultError::Io`] when the file cannot be written; [`VaultError::Corrupt`] if encoding
    /// fails, which would be a bug.
    pub fn save(&self, path: &Path) -> Result<(), VaultError> {
        let tmp = self.stage(path)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// First half of [`Vault::save`]: the new vault, complete and flushed, in a temp file next
    /// to `path`. Until the rename, `path` still holds the old vault.
    fn stage(&self, path: &Path) -> Result<PathBuf, VaultError> {
        let ciphertext = self.to_bytes()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("age.tmp");
        let mut file = private_file(&tmp)?;
        file.write_all(&ciphertext)?;
        // On disk before the rename, or a crash could leave the new name on empty data.
        file.sync_all()?;
        Ok(tmp)
    }

    /// Opens the vault at `path` with `current`, re-seals it under `new` and replaces the file
    /// (temp file and rename). Returns the open vault, now under `new`.
    ///
    /// # Errors
    /// [`VaultError::WrongPassphrase`] when `current` does not open the file, in which case
    /// nothing is written; otherwise as [`Vault::unlock`] and [`Vault::save`].
    pub fn change_passphrase(
        path: &Path,
        current: SecretString,
        new: SecretString,
    ) -> Result<Vault, VaultError> {
        Self::change_passphrase_with(path, current, new, WORK_FACTOR)
    }

    fn change_passphrase_with(
        path: &Path,
        current: SecretString,
        new: SecretString,
        work_factor: u8,
    ) -> Result<Vault, VaultError> {
        let mut vault = Self::unlock(path, current)?;
        vault.passphrase = new;
        vault.work_factor = work_factor;
        vault.save(path)?;
        Ok(vault)
    }

    /// The encrypted vault file, as [`Vault::save`] writes it.
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] if encoding fails, which would be a bug.
    pub fn to_bytes(&self) -> Result<Vec<u8>, VaultError> {
        let contents = Contents {
            version: VERSION,
            entries: self
                .entries
                .iter()
                .map(|(key, value)| Entry {
                    key: key.clone(),
                    value: value.expose_secret().to_owned(),
                })
                .collect(),
        };
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&contents).map_err(|e| VaultError::Corrupt(e.to_string()))?,
        );
        self.seal(&plaintext)
    }

    /// Opens another vault file sealed with the same passphrase (a synced copy).
    ///
    /// # Errors
    /// As [`Vault::unlock_bytes`].
    pub fn reopen(&self, ciphertext: &[u8]) -> Result<Vault, VaultError> {
        Self::unlock_bytes(ciphertext, self.passphrase.clone())
    }

    /// Encrypts any bytes with this vault's passphrase, for data that travels with the vault
    /// (sync uploads hosts and settings this way, so nothing readable leaves the machine).
    ///
    /// # Errors
    /// [`VaultError::Corrupt`] if encryption fails, which would be a bug.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
        let mut recipient = age::scrypt::Recipient::new(self.passphrase.clone());
        recipient.set_work_factor(self.work_factor);
        age::encrypt(&recipient, plaintext).map_err(|e| VaultError::Corrupt(e.to_string()))
    }

    /// Reverses [`Vault::seal`].
    ///
    /// # Errors
    /// [`VaultError::WrongPassphrase`] when sealed under another passphrase or altered.
    pub fn open(&self, ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        let identity = age::scrypt::Identity::new(self.passphrase.clone());
        age::decrypt(&identity, ciphertext)
            .map(Zeroizing::new)
            .map_err(|_| VaultError::WrongPassphrase)
    }

    pub fn get(&self, key: &Key) -> Option<&SecretString> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Stores `value` under `key`, replacing an earlier one.
    pub fn set(&mut self, key: Key, value: SecretString) {
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some((_, slot)) => *slot = value,
            None => self.entries.push((key, value)),
        }
    }

    pub fn remove(&mut self, key: &Key) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(k, _)| k != key);
        self.entries.len() != before
    }

    /// Every key with a secret, in the order they were added. Values stay inside.
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.entries.iter().map(|(k, _)| k)
    }

    /// The passphrase the vault is sealed with, for the Keychain opt-in. Callers must not log
    /// or store it anywhere else.
    pub fn passphrase(&self) -> &SecretString {
        &self.passphrase
    }

    /// An independent copy, so a save can run off the UI thread while the original stays
    /// editable.
    pub fn snapshot(&self) -> Vault {
        Vault {
            passphrase: self.passphrase.clone(),
            entries: self.entries.clone(),
            work_factor: self.work_factor,
        }
    }

    /// Drops every secret and the passphrase now, rather than whenever the value goes away.
    /// They are [`SecretString`]s, zeroed as they are dropped.
    pub fn wipe(&mut self) {
        self.entries.clear();
        self.passphrase = SecretString::from(String::new());
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cheaper scrypt for tests; a real vault keeps [`WORK_FACTOR`].
    #[cfg(test)]
    fn with_work_factor(mut self, log_n: u8) -> Self {
        self.work_factor = log_n;
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn pass(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn login(host: &str) -> Key {
        Key::Password {
            user: "deploy".into(),
            host: host.into(),
            port: 22,
        }
    }

    fn saved(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("vault.age");
        let mut v = Vault::new(pass("correct horse")).with_work_factor(4);
        v.set(login("grape"), pass("hunter2"));
        v.set(
            Key::KeyPassphrase {
                path: "~/.ssh/id_ed25519".into(),
            },
            pass("key-pass"),
        );
        v.save(&path).unwrap();
        path
    }

    #[test]
    fn round_trip_keeps_every_secret_under_its_own_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = saved(&dir);
        let v = Vault::unlock(&path, pass("correct horse")).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v.get(&login("grape")).unwrap().expose_secret(), "hunter2");
        let key = Key::KeyPassphrase {
            path: "~/.ssh/id_ed25519".into(),
        };
        assert_eq!(v.get(&key).unwrap().expose_secret(), "key-pass");
        assert!(v.get(&login("strong-b")).is_none());
    }

    #[test]
    fn file_does_not_contain_the_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = std::fs::read(saved(&dir)).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("hunter2") && !text.contains("grape"));
        assert!(text.starts_with("age-encryption.org/v1"));
    }

    #[test]
    fn wrong_passphrase_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = saved(&dir);
        let err = Vault::unlock(&path, pass("correct horse!")).unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase), "{err:?}");
    }

    #[test]
    fn a_flipped_byte_in_the_body_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = saved(&dir);
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&path, bytes).unwrap();
        assert!(Vault::unlock(&path, pass("correct horse")).is_err());
    }

    #[test]
    fn set_replaces_and_remove_deletes() {
        let mut v = Vault::new(pass("p"));
        v.set(login("grape"), pass("one"));
        v.set(login("grape"), pass("two"));
        assert_eq!(v.len(), 1);
        assert_eq!(v.get(&login("grape")).unwrap().expose_secret(), "two");
        assert!(v.remove(&login("grape")));
        assert!(!v.remove(&login("grape")));
        assert!(v.is_empty());
    }

    #[test]
    fn not_an_age_file_is_corrupt_not_wrong_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.age");
        std::fs::write(&path, b"{\"version\":1}").unwrap();
        let err = Vault::unlock(&path, pass("x")).unwrap_err();
        assert!(matches!(err, VaultError::Corrupt(_)), "{err:?}");
    }

    #[test]
    fn debug_never_shows_secrets() {
        let mut v = Vault::new(pass("master"));
        v.set(login("grape"), pass("hunter2"));
        let shown = format!("{v:?}");
        assert!(!shown.contains("hunter2") && !shown.contains("master"));
    }

    #[test]
    fn sealed_data_opens_only_with_the_same_passphrase() {
        let v = Vault::new(pass("correct horse")).with_work_factor(4);
        let sealed = v.seal(b"hosts.json").unwrap();
        assert!(!String::from_utf8_lossy(&sealed).contains("hosts.json"));
        assert_eq!(&*v.open(&sealed).unwrap(), b"hosts.json");
        let other = Vault::new(pass("battery staple")).with_work_factor(4);
        assert!(matches!(
            other.open(&sealed),
            Err(VaultError::WrongPassphrase)
        ));
    }

    fn ssh_key_entry(name: &str) -> Key {
        Key::SshKey { name: name.into() }
    }

    #[test]
    fn changed_passphrase_opens_the_vault_and_the_old_one_no_longer_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = saved(&dir);
        let rekeyed =
            Vault::change_passphrase_with(&path, pass("correct horse"), pass("battery staple"), 4)
                .unwrap();
        assert_eq!(rekeyed.len(), 2);
        let again = Vault::unlock(&path, pass("battery staple")).unwrap();
        assert_eq!(
            again.get(&login("grape")).unwrap().expose_secret(),
            "hunter2"
        );
        let err = Vault::unlock(&path, pass("correct horse")).unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase), "{err:?}");
    }

    #[test]
    fn wrong_current_passphrase_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = saved(&dir);
        let before = std::fs::read(&path).unwrap();
        let err =
            Vault::change_passphrase_with(&path, pass("not it"), pass("new one"), 4).unwrap_err();
        assert!(matches!(err, VaultError::WrongPassphrase), "{err:?}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(Vault::unlock(&path, pass("correct horse")).is_ok());
    }

    #[test]
    fn a_crash_between_write_and_rename_leaves_the_old_vault_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = saved(&dir);
        let mut next = Vault::unlock(&path, pass("correct horse"))
            .unwrap()
            .with_work_factor(4);
        next.passphrase = pass("battery staple");
        next.set(login("strong-b"), pass("added after"));
        // The process dies here: the new file is fully written but never renamed.
        let tmp = next.stage(&path).unwrap();
        assert!(tmp.exists());
        let old = Vault::unlock(&path, pass("correct horse")).unwrap();
        assert_eq!(old.len(), 2, "old vault must be untouched");
        assert!(Vault::unlock(&path, pass("battery staple")).is_err());
        // Finishing the rename is what makes the new vault current.
        std::fs::rename(&tmp, &path).unwrap();
        let new = Vault::unlock(&path, pass("battery staple")).unwrap();
        assert_eq!(new.len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = saved(&dir);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "mode {mode:o}");
    }

    #[test]
    fn wipe_forgets_every_secret_and_the_passphrase() {
        let mut v = Vault::new(pass("master")).with_work_factor(4);
        v.set(login("grape"), pass("hunter2"));
        v.set(ssh_key_entry("laptop"), pass("pem"));
        v.wipe();
        assert!(v.is_empty());
        assert!(v.get(&login("grape")).is_none());
        assert_eq!(v.passphrase().expose_secret(), "");
    }

    #[test]
    fn snapshot_is_independent_of_the_original() {
        let mut v = Vault::new(pass("master"));
        v.set(login("grape"), pass("hunter2"));
        let copy = v.snapshot();
        v.remove(&login("grape"));
        assert_eq!(
            copy.get(&login("grape")).unwrap().expose_secret(),
            "hunter2"
        );
    }

    #[test]
    fn keys_lists_every_entry_kind_in_insertion_order() {
        let mut v = Vault::new(pass("p"));
        v.set(login("grape"), pass("hunter2"));
        v.set(ssh_key_entry("laptop"), pass("pem"));
        let keys: Vec<_> = v.keys().cloned().collect();
        assert_eq!(keys, vec![login("grape"), ssh_key_entry("laptop")]);
    }

    #[test]
    fn generated_key_round_trips_through_the_vault_and_reports_its_public_half() {
        let g = keys::generate_ed25519("tern@test").unwrap();
        assert!(
            g.private
                .expose_secret()
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert!(g.info.public.starts_with("ssh-ed25519 "));
        assert!(g.info.public.ends_with(" tern@test"));
        assert!(g.info.fingerprint.starts_with("SHA256:"));
        // Two keys are never the same key.
        let other = keys::generate_ed25519("tern@test").unwrap();
        assert_ne!(g.info.public, other.info.public);

        let mut v = Vault::new(pass("p")).with_work_factor(4);
        let key = ssh_key_entry("laptop");
        v.set(key.clone(), g.private);
        let bytes = v.to_bytes().unwrap();
        let back = v.reopen(&bytes).unwrap();
        let info = keys::inspect(back.get(&key).unwrap().expose_secret()).unwrap();
        assert_eq!(info, g.info);
    }

    /// A vault file whose plaintext is `contents`, sealed under `passphrase`.
    fn sealed_contents(contents: &Contents, passphrase: &str) -> Vec<u8> {
        let plaintext = serde_json::to_vec(contents).unwrap();
        Vault::new(pass(passphrase))
            .with_work_factor(4)
            .seal(&plaintext)
            .unwrap()
    }

    mod hostile {
        use std::collections::HashMap;

        use proptest::prelude::*;

        use super::*;

        /// Unique non-empty names; the name's length picks which kind of key it becomes, so both
        /// kinds are exercised and uniqueness of the name keeps the keys unique.
        fn key_for(name: &str) -> Key {
            if name.chars().count().is_multiple_of(2) {
                Key::SshKey { name: name.into() }
            } else {
                Key::KeyPassphrase { path: name.into() }
            }
        }

        fn entries() -> impl Strategy<Value = HashMap<String, String>> {
            // `\PC` is any printable char (unicode); the literals add whitespace and controls.
            let text = prop_oneof![
                "\\PC{0,24}",
                "[ \\t\\n\\r]{0,6}\\PC{0,6}[ \\t\\n\\r]{0,6}",
                Just("pässwörd 密码 🔑\n".to_owned()),
            ];
            let name = prop_oneof!["\\PC{1,16}", "[ \\t]{1,4}x[ \\t]{0,4}"];
            prop::collection::hash_map(name, text, 0..6)
        }

        fn passphrase() -> impl Strategy<Value = String> {
            "\\PC{1,16}"
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(24))]

            #[test]
            fn round_trip_returns_exactly_the_entries_put_in(
                map in entries(),
                phrase in passphrase(),
            ) {
                let mut v = Vault::new(pass(&phrase)).with_work_factor(4);
                for (name, value) in &map {
                    v.set(key_for(name), pass(value));
                }
                let bytes = v.to_bytes().unwrap();

                let back = Vault::unlock_bytes(&bytes, pass(&phrase)).unwrap();

                prop_assert_eq!(back.len(), map.len());
                for (name, value) in &map {
                    let got = back.get(&key_for(name));
                    prop_assert_eq!(got.map(|s| s.expose_secret().to_owned()), Some(value.clone()));
                }
            }

            #[test]
            fn another_passphrase_is_wrong_passphrase_not_corrupt(
                map in entries(),
                right in passphrase(),
                wrong in passphrase(),
            ) {
                prop_assume!(right != wrong);
                let mut v = Vault::new(pass(&right)).with_work_factor(4);
                for (name, value) in &map {
                    v.set(key_for(name), pass(value));
                }
                let bytes = v.to_bytes().unwrap();

                let err = Vault::unlock_bytes(&bytes, pass(&wrong)).unwrap_err();

                prop_assert!(matches!(err, VaultError::WrongPassphrase), "{:?}", err);
            }

            #[test]
            fn any_truncation_of_a_real_vault_is_refused(
                map in entries(),
                cut in 0.0f64..1.0,
            ) {
                let mut v = Vault::new(pass("correct horse")).with_work_factor(4);
                for (name, value) in &map {
                    v.set(key_for(name), pass(value));
                }
                let bytes = v.to_bytes().unwrap();
                let keep = (bytes.len() as f64 * cut) as usize; // strictly shorter than `bytes`

                prop_assert!(Vault::unlock_bytes(&bytes[..keep], pass("correct horse")).is_err());
            }

            #[test]
            fn arbitrary_bytes_are_refused(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
                prop_assert!(Vault::unlock_bytes(&bytes, pass("x")).is_err());
            }

            #[test]
            fn arbitrary_bytes_behind_a_valid_age_envelope_are_corrupt(
                bytes in prop::collection::vec(any::<u8>(), 0..128),
            ) {
                let sealed = Vault::new(pass("p")).with_work_factor(4).seal(&bytes).unwrap();

                let err = Vault::unlock_bytes(&sealed, pass("p")).unwrap_err();

                prop_assert!(matches!(err, VaultError::Corrupt(_)), "{:?}", err);
            }
        }

        #[test]
        fn valid_json_of_the_wrong_shape_is_corrupt() {
            for plaintext in [&b"{\"version\":1}"[..], b"[]", b"null", b"{\"version\":\"1\",\"entries\":[]}"] {
                let sealed = Vault::new(pass("p"))
                    .with_work_factor(4)
                    .seal(plaintext)
                    .unwrap();
                let err = Vault::unlock_bytes(&sealed, pass("p")).unwrap_err();
                assert!(matches!(err, VaultError::Corrupt(_)), "{plaintext:?}: {err:?}");
            }
        }

        #[test]
        fn a_newer_vault_version_is_refused_naming_the_version() {
            let newer = Contents {
                version: VERSION + 1,
                entries: vec![Entry {
                    key: login("grape"),
                    value: "hunter2".into(),
                }],
            };
            let bytes = sealed_contents(&newer, "correct horse");

            let err = Vault::unlock_bytes(&bytes, pass("correct horse")).unwrap_err();

            let VaultError::Corrupt(msg) = err else {
                panic!("expected Corrupt, got {err:?}");
            };
            assert!(msg.contains("version"), "{msg}");
            assert!(msg.contains(&(VERSION + 1).to_string()), "{msg}");
        }

        #[test]
        fn an_older_vault_version_is_refused_too() {
            let older = Contents {
                version: VERSION - 1,
                entries: Vec::new(),
            };
            let bytes = sealed_contents(&older, "p");
            let err = Vault::unlock_bytes(&bytes, pass("p")).unwrap_err();
            assert!(matches!(err, VaultError::Corrupt(_)), "{err:?}");
        }

        #[test]
        fn a_failed_load_never_touches_the_file_on_disk() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("vault.age");
            let newer = Contents {
                version: VERSION + 1,
                entries: Vec::new(),
            };
            let bytes = sealed_contents(&newer, "correct horse");
            std::fs::write(&path, &bytes).unwrap();

            assert!(Vault::unlock(&path, pass("correct horse")).is_err());
            assert!(Vault::unlock(&path, pass("wrong")).is_err());
            // Re-keying must fail at the open step, before anything is written.
            let err =
                Vault::change_passphrase_with(&path, pass("correct horse"), pass("new"), 4)
                    .unwrap_err();
            assert!(matches!(err, VaultError::Corrupt(_)), "{err:?}");

            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert!(!path.with_extension("age.tmp").exists());
        }
    }

    #[test]
    fn inspect_refuses_text_that_is_not_a_usable_key() {
        assert!(matches!(
            keys::inspect("hello"),
            Err(keys::KeyError::NotAKey)
        ));
        let g = keys::generate_ed25519("x").unwrap();
        // The public line is not a private key.
        assert!(matches!(
            keys::inspect(&g.info.public),
            Err(keys::KeyError::NotAKey)
        ));
    }
}

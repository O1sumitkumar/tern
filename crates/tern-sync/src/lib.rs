//! Sync through a private GitHub gist described `tern-sync`, as Tabby syncs its settings.
//!
//! What is uploaded: the vault file as it is (already age-encrypted), and the other files
//! (hosts, settings, keymap) sealed with the vault passphrase by the caller, plus a small
//! plain `tern-meta.json` holding the bundle hash and time. Nothing readable leaves the Mac.
//!
//! Deciding what to do needs no network: compare the local and remote bundle hashes with the
//! hash both sides had at the last sync ([`decide`]).

mod github;
mod gitrepo;
mod merge;

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

pub use github::{Gist, GithubError, RemoteBundle, TokenSource, forget_token, save_token, token};
pub use gitrepo::{GitRepo, create_github_repo};
pub use merge::{ABSENT, HostsMerge, ItemPlan, decide_item, item_hashes, merge_hosts};

/// The files that travel, by name, as raw bytes.
pub type Files = BTreeMap<String, Vec<u8>>;

/// A stable fingerprint of a bundle: SHA-256 over name, length and bytes of each file in
/// name order, so renaming, reordering or moving bytes between files all change it.
pub fn hash(files: &Files) -> String {
    let mut h = Sha256::new();
    for (name, bytes) in files {
        h.update(name.as_bytes());
        h.update([0]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    UpToDate,
    Push,
    Pull,
    /// Both sides changed since the last sync: the user picks which one wins.
    Conflict,
}

/// What a sync should do. `last` is the hash both sides agreed on at the previous sync,
/// `None` on a Mac that never synced.
pub fn decide(local: &str, remote: Option<&str>, last: Option<&str>) -> Plan {
    let Some(remote) = remote else {
        return Plan::Push;
    };
    if local == remote {
        return Plan::UpToDate;
    }
    match last {
        // First sync on this Mac and the two differ: neither side can be called stale.
        None => Plan::Conflict,
        Some(last) => match (local != last, remote != last) {
            (true, true) => Plan::Conflict,
            (true, false) => Plan::Push,
            (false, true) => Plan::Pull,
            (false, false) => Plan::UpToDate,
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn files(pairs: &[(&str, &str)]) -> Files {
        pairs
            .iter()
            .map(|(n, b)| ((*n).to_owned(), b.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn the_decision_table() {
        assert_eq!(decide("a", None, None), Plan::Push);
        assert_eq!(decide("a", Some("a"), None), Plan::UpToDate);
        assert_eq!(decide("a", Some("b"), None), Plan::Conflict);
        assert_eq!(decide("new", Some("old"), Some("old")), Plan::Push);
        assert_eq!(decide("old", Some("new"), Some("old")), Plan::Pull);
        assert_eq!(decide("x", Some("y"), Some("old")), Plan::Conflict);
        assert_eq!(decide("same", Some("same"), Some("old")), Plan::UpToDate);
    }

    #[test]
    fn hash_sees_content_names_and_boundaries() {
        let base = hash(&files(&[("a", "12"), ("b", "3")]));
        assert_eq!(
            base,
            hash(&files(&[("b", "3"), ("a", "12")])),
            "map order is canonical"
        );
        assert_ne!(base, hash(&files(&[("a", "1"), ("b", "23")])), "moved byte");
        assert_ne!(
            base,
            hash(&files(&[("a", "12"), ("c", "3")])),
            "renamed file"
        );
        assert_ne!(
            base,
            hash(&files(&[("a", "12"), ("b", "4")])),
            "changed byte"
        );
        assert_eq!(base.len(), 64);
    }

    /// No files hashes as SHA-256 of zero bytes, the standard empty-input vector.
    #[test]
    fn the_empty_bundle_has_the_sha256_of_no_input() {
        assert_eq!(
            hash(&Files::new()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// A file that exists but is empty is a different bundle from no file at all, so a sync
    /// cannot mistake "the user cleared this file" for "this file was never there".
    #[test]
    fn a_present_empty_file_is_not_an_absent_file() {
        assert_ne!(hash(&files(&[("hosts.json", "")])), hash(&Files::new()));
    }

    /// The fingerprint is persisted in `tern-meta.json` and compared across Macs and app
    /// versions, so its byte layout is a wire format. This pins it: for each file, the name,
    /// a 0 byte, the length as u64 little-endian, then the bytes, hashed in name order. The
    /// expected digest was computed outside this crate with `shasum -a 256` over those bytes.
    #[test]
    fn a_two_file_bundle_matches_its_documented_layout() {
        let bundle = files(&[("settings.json", "{}"), ("hosts.json", "[]")]);
        assert_eq!(
            hash(&bundle),
            "4583a05901e21bba5a8c695e214b9dd4f34e2b0dce8b261318afc15a8312cff5"
        );
    }

    use proptest::prelude::*;

    /// A bundle of one to four config-like files, each with non-empty printable contents.
    fn any_bundle() -> impl Strategy<Value = Files> {
        prop::collection::btree_map(
            "[a-z]{1,8}\\.json",
            "[ -~]{1,64}".prop_map(String::into_bytes),
            1..5,
        )
    }

    proptest! {
        /// Flipping one bit of one byte, in any file at any offset, changes the fingerprint.
        #[test]
        fn any_single_changed_byte_changes_the_fingerprint(
            bundle in any_bundle(),
            which_file in any::<prop::sample::Index>(),
            which_byte in any::<prop::sample::Index>(),
        ) {
            let names: Vec<String> = bundle.keys().cloned().collect();
            let name = &names[which_file.index(names.len())];
            let mut changed = bundle.clone();
            let bytes = changed.get_mut(name).unwrap();
            let at = which_byte.index(bytes.len());
            bytes[at] ^= 0x01;
            prop_assert_ne!(hash(&bundle), hash(&changed));
        }

        /// The same content divided between two files at a different point is a different
        /// bundle: the fingerprint sees where one file ends and the next begins.
        #[test]
        fn moving_the_split_point_changes_the_fingerprint(
            content in "[a-z]{2,40}",
            first_cut in any::<prop::sample::Index>(),
            second_cut in any::<prop::sample::Index>(),
        ) {
            let len = content.len();
            let i = 1 + first_cut.index(len - 1);
            let j = 1 + second_cut.index(len - 1);
            prop_assume!(i != j);
            let split_at = |k: usize| files(&[("a.json", &content[..k]), ("b.json", &content[k..])]);
            prop_assert_ne!(hash(&split_at(i)), hash(&split_at(j)));
        }
    }
}

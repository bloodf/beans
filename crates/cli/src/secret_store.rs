//! Inert secret-store contract. Production continues to use private JSON files.
//! Migration copies and verifies bytes; it never authorizes deleting the source.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretKind {
    MasterSecret,
    Credentials,
}

/// Namespace must bind the admitted home and account before backend activation.
#[derive(Clone, Copy)]
pub struct SecretId<'a> {
    pub namespace: &'a str,
    pub kind: SecretKind,
}

/// Errors contain no backend messages or secret bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Locked,
    Unavailable,
    AmbiguousWrite,
    Conflict,
    VerificationFailed,
    InvalidNamespace,
}

/// Native adapters must distinguish missing entries from locked/unavailable stores.
pub trait SecretStore {
    fn read(&self, id: SecretId<'_>) -> Result<Option<Vec<u8>>, StoreError>;

    /// Atomically create only if absent. Never replace a conflicting entry.
    /// A timeout after publication returns AmbiguousWrite, not success.
    fn insert(&mut self, id: SecretId<'_>, bytes: &[u8]) -> Result<(), StoreError>;
}

/// The source and its backup remain owned by the caller, including on success.
/// Call under exclusive account lifecycle admission. Restart repeats verification.
pub fn copy_verified(
    store: &mut impl SecretStore,
    id: SecretId<'_>,
    source: &[u8],
) -> Result<(), StoreError> {
    if id.namespace.is_empty() {
        return Err(StoreError::InvalidNamespace);
    }
    match store.read(id)? {
        Some(existing) if existing != source => return Err(StoreError::Conflict),
        Some(_) => {}
        None => store.insert(id, source)?,
    }
    match store.read(id)? {
        Some(stored) if stored == source => Ok(()),
        _ => Err(StoreError::VerificationFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct Fake {
        value: Option<Vec<u8>>,
        failure: Option<StoreError>,
        ambiguous: bool,
        corrupt: bool,
        reads: Cell<usize>,
    }

    impl SecretStore for Fake {
        fn read(&self, _: SecretId<'_>) -> Result<Option<Vec<u8>>, StoreError> {
            self.reads.set(self.reads.get() + 1);
            if let Some(error) = self.failure {
                return Err(error);
            }
            Ok(self.value.clone())
        }

        fn insert(&mut self, _: SecretId<'_>, bytes: &[u8]) -> Result<(), StoreError> {
            if self.value.is_some() {
                return Err(StoreError::Conflict);
            }
            self.value = Some(if self.corrupt { vec![] } else { bytes.to_vec() });
            if self.ambiguous {
                return Err(StoreError::AmbiguousWrite);
            }
            Ok(())
        }
    }

    fn id(kind: SecretKind) -> SecretId<'static> {
        SecretId {
            namespace: "temporary-test-account",
            kind,
        }
    }

    #[test]
    fn copy_and_restart_verify_both_secret_kinds() {
        for kind in [SecretKind::MasterSecret, SecretKind::Credentials] {
            let mut store = Fake::default();
            let source = b"synthetic fixture".to_vec();
            assert_eq!(copy_verified(&mut store, id(kind), &source), Ok(()));
            assert_eq!(store.value.as_deref(), Some(source.as_slice()));
            assert_eq!(copy_verified(&mut store, id(kind), &source), Ok(()));
            assert_eq!(store.reads.get(), 4);
            assert_eq!(source, b"synthetic fixture");
        }
    }

    #[test]
    fn locked_and_headless_unavailable_never_write() {
        for error in [StoreError::Locked, StoreError::Unavailable] {
            let mut store = Fake {
                failure: Some(error),
                ..Fake::default()
            };
            assert_eq!(
                copy_verified(&mut store, id(SecretKind::Credentials), b"fixture"),
                Err(error)
            );
            assert_eq!(store.value, None);
        }
    }

    #[test]
    fn ambiguous_publication_requires_restart_verification() {
        let mut store = Fake {
            ambiguous: true,
            ..Fake::default()
        };
        let key = id(SecretKind::MasterSecret);
        assert_eq!(
            copy_verified(&mut store, key, b"fixture"),
            Err(StoreError::AmbiguousWrite)
        );
        assert_eq!(store.value.as_deref(), Some(b"fixture".as_slice()));
        assert_eq!(copy_verified(&mut store, key, b"fixture"), Ok(()));
    }

    #[test]
    fn conflicting_and_corrupt_entries_fail_closed() {
        let mut store = Fake {
            value: Some(b"other".to_vec()),
            ..Fake::default()
        };
        let key = id(SecretKind::Credentials);
        assert_eq!(
            copy_verified(&mut store, key, b"fixture"),
            Err(StoreError::Conflict)
        );
        assert_eq!(store.value.as_deref(), Some(b"other".as_slice()));
        let mut store = Fake {
            corrupt: true,
            ..Fake::default()
        };
        assert_eq!(
            copy_verified(&mut store, key, b"fixture"),
            Err(StoreError::VerificationFailed)
        );
    }

    #[test]
    fn empty_namespace_has_no_backend_effects() {
        let mut store = Fake::default();
        let key = SecretId {
            namespace: "",
            kind: SecretKind::MasterSecret,
        };
        assert_eq!(
            copy_verified(&mut store, key, b"fixture"),
            Err(StoreError::InvalidNamespace)
        );
        assert_eq!(store.reads.get(), 0);
        assert_eq!(store.value, None);
    }
}

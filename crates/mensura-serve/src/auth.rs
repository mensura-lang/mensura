//! Principals and scopes (ADR 0045 decision 8): static bearer tokens
//! matched by SHA-256 digest, as an interim subset of ADR 0005.  A
//! request presents `Authorization: Bearer <token>`; the instance hashes
//! it, finds the principal, and checks the operation's scope.

use std::collections::HashMap;

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

use crate::config::Principal;

/// The configured principals, indexed by token digest.
#[derive(Clone, Debug, Default)]
pub struct Principals {
    by_digest: HashMap<String, Principal>,
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// No credential, or one that matches no principal: a 401.
    Unauthenticated,
    /// A known principal without the permission: a 403.
    Forbidden { principal: String, scope: String },
}

impl Principals {
    pub fn new(principals: &[Principal]) -> Principals {
        Principals {
            by_digest: principals
                .iter()
                .map(|p| (p.token_sha256.to_ascii_lowercase(), p.clone()))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_digest.is_empty()
    }

    /// The principal a request's bearer token names.
    pub fn authenticate(&self, headers: &HeaderMap) -> Result<&Principal, AuthError> {
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or(AuthError::Unauthenticated)?;
        self.by_digest
            .get(&digest(token))
            .ok_or(AuthError::Unauthenticated)
    }
}

impl Principal {
    /// Whether this principal holds `scope`, directly or through the
    /// operation's `all` wildcard (`read:all`, `write:all`, `floor:all`).
    pub fn allows(&self, scope: &str) -> bool {
        let wildcard = scope.split_once(':').map(|(op, _)| format!("{op}:all"));
        self.scopes
            .iter()
            .any(|s| s == scope || wildcard.as_deref() == Some(s.as_str()))
    }

    /// Check `scope`, naming the principal in the refusal.
    pub fn require(&self, scope: &str) -> Result<(), AuthError> {
        if self.allows(scope) {
            Ok(())
        } else {
            Err(AuthError::Forbidden {
                principal: self.id.clone(),
                scope: scope.to_string(),
            })
        }
    }
}

/// The hexadecimal SHA-256 digest of a token, as the configuration
/// records it.
pub fn digest(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The wire form of a declared name (`docs/language/05-naming-and-casing.md`):
/// one kebab-case segment, which is also a scope's resource half.
pub fn wire_segment(name: &str) -> String {
    name.replace('_', "-")
}

/// The scope an operation on a table needs: `read:<resource>`,
/// `write:<resource>`, or `floor:<resource>`.
pub fn scope(op: &str, name: &str) -> String {
    format!("{op}:{}", wire_segment(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(scopes: &[&str]) -> Principal {
        Principal {
            id: "spiffe://local/service/test".to_string(),
            token_sha256: digest("secret"),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn headers(value: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(v) = value {
            h.insert(axum::http::header::AUTHORIZATION, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn a_token_is_matched_by_digest() {
        let principals = Principals::new(&[principal(&["read:readings"])]);
        assert!(principals.authenticate(&headers(None)).is_err());
        assert!(
            principals
                .authenticate(&headers(Some("Bearer nope")))
                .is_err()
        );
        assert!(
            principals
                .authenticate(&headers(Some("Basic secret")))
                .is_err()
        );
        let p = principals
            .authenticate(&headers(Some("Bearer secret")))
            .unwrap();
        assert_eq!(p.id, "spiffe://local/service/test");
    }

    #[test]
    fn scopes_match_directly_or_by_wildcard() {
        let p = principal(&["read:all", "write:temperature-readings"]);
        assert!(p.allows("read:readings"));
        assert!(p.allows(&scope("write", "temperature_readings")));
        assert!(!p.allows("write:readings"));
        assert!(!p.allows("floor:readings"));
        let err = p.require("floor:readings").unwrap_err();
        assert!(matches!(err, AuthError::Forbidden { .. }));
    }

    #[test]
    fn the_digest_is_the_standard_hex_sha256() {
        assert_eq!(
            digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

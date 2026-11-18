//! Paste identifiers.
//!
//! An id is exactly 128 bits of client-generated randomness, encoded as
//! base64url without padding — 22 characters. The server accepts nothing else.
//!
//! Why the strictness matters: the id is the only thing an attacker can guess.
//! Accepting shorter ids, or ids in a second encoding, would create a smaller
//! search space and two spellings of the same paste. The parser therefore
//! rejects rather than normalises.

use core::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Raw length of an id in bytes.
pub const ID_BYTES: usize = 16;

/// Length of an id in its base64url representation.
pub const ID_CHARS: usize = 22;

/// A validated paste id.
///
/// Constructing one is the only way to get an id into the rest of the system,
/// so a value of this type is proof that the format has been checked.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PasteId([u8; ID_BYTES]);

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("id must be {ID_CHARS} characters, got {0}")]
    Length(usize),
    #[error("id is not base64url without padding")]
    Encoding,
    /// Reachable only for inputs whose length is right but which decode to the
    /// wrong number of bytes. Kept explicit so the parser has no silent path.
    #[error("id must decode to {ID_BYTES} bytes, got {0}")]
    DecodedLength(usize),
}

impl PasteId {
    /// Parses the canonical representation.
    pub fn parse(text: &str) -> Result<Self, IdError> {
        // Check the length first so a huge input cannot reach the decoder.
        if text.len() != ID_CHARS {
            return Err(IdError::Length(text.len()));
        }

        let decoded = URL_SAFE_NO_PAD.decode(text.as_bytes()).map_err(|_| IdError::Encoding)?;

        let bytes: [u8; ID_BYTES] = decoded.as_slice().try_into().map_err(|_| IdError::DecodedLength(decoded.len()))?;

        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: [u8; ID_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; ID_BYTES] {
        &self.0
    }

    /// The canonical string form.
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    /// A shortened form for debug logging.
    ///
    /// Full ids must never reach a log, but during debugging it helps to
    /// correlate two lines. Eight characters are enough for that and far too few
    /// to reconstruct the id.
    pub fn prefix(&self) -> String {
        self.encode().chars().take(8).collect()
    }
}

impl fmt::Display for PasteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

/// Deliberately not the full id: `Debug` output ends up in error chains and log
/// lines by accident, which is exactly what must not happen here.
impl fmt::Debug for PasteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PasteId({}…)", self.prefix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canonical id. Note the final `A`: 22 base64url characters carry 132
    /// bits, so the last character's four unused low bits must be zero. Only
    /// `A`, `Q`, `g` and `w` satisfy that.
    const VALID: &str = "2pS3xQ7Rk9mNpLz4AbCdEA";

    #[test]
    fn accepts_the_canonical_form() {
        let id = PasteId::parse(VALID).expect("valid id");
        assert_eq!(id.encode(), VALID);
    }

    #[test]
    fn round_trips_through_bytes() {
        let id = PasteId::parse(VALID).expect("valid id");
        assert_eq!(PasteId::from_bytes(*id.as_bytes()), id);
    }

    /// Everything that is not exactly the canonical 22-character base64url form
    /// has to be rejected. Padding and the standard base64 alphabet are the
    /// interesting cases: accepting them would give one paste two spellings.
    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "short",
            "2pS3xQ7Rk9mNpLz4AbCdE",   // 21 characters
            "2pS3xQ7Rk9mNpLz4AbCdEAG", // 23 characters
            "2pS3xQ7Rk9mNpLz4AbCd==",  // padding
            "2pS3xQ7Rk9mNpLz4AbC+EA",  // standard base64 alphabet
            "2pS3xQ7Rk9mNpLz4AbC/EA",  // standard base64 alphabet
            "2pS3xQ7Rk9mNpLz4AbCdA ",  // trailing space
            "../../../etc/passwd0EA",  // path traversal at the right length
            "2pS3xQ7Rk9mNpLz4AbCdÄA",  // non-ASCII
        ] {
            assert!(PasteId::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    /// The subtle one. 22 base64url characters carry 132 bits but an id is 128,
    /// so the final character has four unused bits. If the decoder ignored them,
    /// sixteen different strings would map to the same id - sixteen spellings of
    /// one paste, and an id space that is quietly larger than it looks. The
    /// decoder must reject any final character whose low bits are set.
    #[test]
    fn rejects_non_canonical_trailing_bits() {
        let canonical = "2pS3xQ7Rk9mNpLz4AbCdEA";
        assert!(PasteId::parse(canonical).is_ok());

        // Same leading 21 characters, final character with stray low bits.
        for suffix in ['B', 'C', 'f', 'x', '9', '-', '_'] {
            let mutated = format!("2pS3xQ7Rk9mNpLz4AbCdE{suffix}");
            assert!(
                PasteId::parse(&mutated).is_err(),
                "{mutated} has non-zero trailing bits and must be rejected"
            );
        }

        // The four characters that are legal in the final position all differ.
        let mut canonical_ids = std::collections::HashSet::new();
        for suffix in ['A', 'Q', 'g', 'w'] {
            let id = PasteId::parse(&format!("2pS3xQ7Rk9mNpLz4AbCdE{suffix}")).expect("canonical trailing character");
            assert!(canonical_ids.insert(*id.as_bytes()), "ids collided");
        }
        assert_eq!(canonical_ids.len(), 4);
    }

    #[test]
    fn debug_and_prefix_do_not_reveal_the_full_id() {
        let id = PasteId::parse(VALID).expect("valid id");

        assert_eq!(id.prefix().len(), 8);
        assert!(!id.prefix().contains("Lz4"));

        let debug = format!("{id:?}");
        assert!(!debug.contains(VALID), "Debug leaked the full id: {debug}");
    }
}

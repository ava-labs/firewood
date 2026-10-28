// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

/// Opaque-by-convention state for carrying a proof-sizing hint across C calls.
///
/// Pass the bytes back to Firewood unchanged. The encoding is versioned and is
/// not the in-memory representation of [`firewood::SizingHint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SizedProofHint {
    /// Internal state; callers must preserve these bytes unchanged.
    pub internal_state: [u8; 22],
}

const _: [(); firewood::SIZING_HINT_ENCODING_LEN] = [(); 22];

impl From<firewood::SizingHint> for SizedProofHint {
    fn from(hint: firewood::SizingHint) -> Self {
        Self {
            internal_state: hint.into(),
        }
    }
}

impl TryFrom<SizedProofHint> for firewood::SizingHint {
    type Error = firewood::InvalidSizingHintEncoding;

    fn try_from(hint: SizedProofHint) -> Result<Self, Self::Error> {
        Self::try_from(hint.internal_state)
    }
}

#[cfg(test)]
mod tests {
    use super::SizedProofHint;

    #[test]
    fn sizing_hint_ffi_round_trips() {
        let hint = firewood::SizingHint::new(2048);
        let ffi_hint = SizedProofHint::from(hint);
        assert_eq!(firewood::SizingHint::try_from(ffi_hint), Ok(hint));
    }

    #[test]
    fn sizing_hint_ffi_size_matches_encoding() {
        assert_eq!(
            std::mem::size_of::<SizedProofHint>(),
            firewood::SIZING_HINT_ENCODING_LEN
        );
    }
}

// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    Children, HashType, Hashable, HashableShunt, HashedTrieNode, JoinedPath, PackedPathRef,
    PathBuf, PathComponent, PathGuard, SplitPath, TrieNode, TriePath, TriePathFromPackedBytes,
    ValueDigest,
};

/// A duplicate key error when merging two key-value tries.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
#[error("duplicate keys found at path {}", path.display())]
pub struct DuplicateKeyError {
    /// The path at which the duplicate keys were found.
    pub path: PathBuf,
}

/// The root of a key-value trie.
///
/// The trie maps byte string keys to values of type `T`.
///
/// The trie is a compressed prefix tree, where each node contains:
/// - A partial path (a sequence of path components) representing the path from
///   its parent to itself.
/// - An optional value of type `T` if the path to this node corresponds to a
///   key in the trie.
/// - A set of children nodes, each corresponding to a different path component
///   extending the current node's path.
pub struct KeyValueTrieRoot<'a, T: ?Sized> {
    /// The partial path from this node's parent to itself.
    pub partial_path: PackedPathRef<'a>,
    /// The value associated with the path to this node, if any.
    ///
    /// If [`None`], this node does not correspond to a key in the trie and is
    /// expected to have two or more children. If [`Some`], this node may have
    /// zero or more children.
    pub value: Option<&'a T>,
    /// The children of this node, indexed by their leading path component.
    pub children: Children<Option<Box<Self>>>,
}

/// The root of a hashed key-value trie.
///
/// This is similar to [`KeyValueTrieRoot`], but includes the computed hash of
/// the node as well as its leading path components. Consequently, the hashed
/// trie is formed by hashing the un-hashed trie.
pub struct HashedKeyValueTrieRoot<'a, T: ?Sized> {
    computed: HashType,
    leading_path: PathBuf,
    partial_path: PackedPathRef<'a>,
    value: Option<&'a T>,
    children: Children<Option<Box<Self>>>,
}

impl<T: AsRef<[u8]> + ?Sized> std::fmt::Debug for KeyValueTrieRoot<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyValueTrieRoot")
            .field("partial_path", &self.partial_path.display())
            .field("value", &DebugValue::new(self.value))
            .field("children", &DebugChildren::new(&self.children))
            .finish()
    }
}

impl<T: AsRef<[u8]> + ?Sized> std::fmt::Debug for HashedKeyValueTrieRoot<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashedKeyValueTrieRoot")
            .field("computed", &self.computed)
            .field("leading_path", &self.leading_path.display())
            .field("partial_path", &self.partial_path.display())
            .field("value", &DebugValue::new(self.value))
            .field("children", &DebugChildren::new(&self.children))
            .finish()
    }
}

impl<'a, T: AsRef<[u8]> + ?Sized> KeyValueTrieRoot<'a, T> {
    /// Constructs a new leaf node with the given path and value.
    #[must_use]
    pub fn new_leaf(path: &'a [u8], value: &'a T) -> Box<Self> {
        Box::new(Self {
            partial_path: PackedPathRef::path_from_packed_bytes(path),
            value: Some(value),
            children: Children::new(),
        })
    }

    /// Constructs a new key-value trie from the given slice of key-value pairs.
    ///
    /// For efficiency, the slice should be sorted by key; however, this is not
    /// explicitly required.
    ///
    /// # Errors
    ///
    /// If duplicate keys are found, a [`DuplicateKeyError`] is returned.
    pub fn from_slice<K, V>(slice: &'a [(K, V)]) -> Result<Option<Box<Self>>, DuplicateKeyError>
    where
        K: AsRef<[u8]>,
        V: AsRef<T>,
    {
        match slice {
            [] => Ok(None),
            [(k, v)] => Ok(Some(Self::new_leaf(k.as_ref(), v.as_ref()))),
            _ => {
                let mid = slice.len() / 2;
                let (lhs, rhs) = slice.split_at(mid);
                let lhs = Self::from_slice(lhs)?;
                let rhs = Self::from_slice(rhs)?;
                Self::merge_root(PathGuard::new(&mut PathBuf::new_const()), lhs, rhs)
            }
        }
    }

    /// Constructs a new internal node with the given two children.
    ///
    /// The two children must have different leading path components.
    #[must_use]
    fn new_siblings(
        path: PackedPathRef<'a>,
        lhs_path: PathComponent,
        lhs: Box<Self>,
        rhs_path: PathComponent,
        rhs: Box<Self>,
    ) -> Box<Self> {
        debug_assert_ne!(lhs_path, rhs_path);
        let mut children = Children::new();
        children[lhs_path] = Some(lhs);
        children[rhs_path] = Some(rhs);
        Box::new(Self {
            partial_path: path,
            value: None,
            children,
        })
    }

    /// Returns a new node with the same contents as `self` but with its path
    /// replaced by the given path.
    #[must_use]
    fn with_path(mut self: Box<Self>, path: PackedPathRef<'a>) -> Box<Self> {
        self.partial_path = path;
        self
    }

    /// Deeply merges two key-value tries, returning the merged trie.
    fn merge_root(
        leading_path: PathGuard<'_>,
        lhs: Option<Box<Self>>,
        rhs: Option<Box<Self>>,
    ) -> Result<Option<Box<Self>>, DuplicateKeyError> {
        match (lhs, rhs) {
            (Some(l), Some(r)) => Self::merge(leading_path, l, r).map(Some),
            (Some(node), None) | (None, Some(node)) => Ok(Some(node)),
            (None, None) => Ok(None),
        }
    }

    /// Merges two disjoint tries, returning the merged trie.
    fn merge(
        leading_path: PathGuard<'_>,
        lhs: Box<Self>,
        rhs: Box<Self>,
    ) -> Result<Box<Self>, DuplicateKeyError> {
        match lhs
            .partial_path
            .longest_common_prefix(rhs.partial_path)
            .split_first_parts()
        {
            (None, None, _) => {
                // both paths are identical, perform a deep merge of each child slot
                Self::deep_merge(leading_path, lhs, rhs)
            }
            (Some((l, l_rest)), Some((r, r_rest)), path) => {
                // the paths diverge at this component, create a new parent node
                // with the two nodes as children
                Ok(Self::new_siblings(
                    path,
                    l,
                    lhs.with_path(l_rest),
                    r,
                    rhs.with_path(r_rest),
                ))
            }
            (Some((l, l_rest)), None, _) => {
                // rhs is a prefix of lhs, so rhs becomes the parent of lhs
                rhs.merge_child(leading_path, l, lhs.with_path(l_rest))
            }
            (None, Some((r, r_rest)), _) => {
                // lhs is a prefix of rhs, so lhs becomes the parent of rhs
                lhs.merge_child(leading_path, r, rhs.with_path(r_rest))
            }
        }
    }

    /// Deeply merges two kvp nodes that have identical paths.
    fn deep_merge(
        mut leading_path: PathGuard<'_>,
        mut lhs: Box<Self>,
        #[expect(clippy::boxed_local)] mut rhs: Box<Self>,
    ) -> Result<Box<Self>, DuplicateKeyError> {
        leading_path.extend(lhs.partial_path.components());

        lhs.value = match (lhs.value.take(), rhs.value.take()) {
            (Some(v), None) | (None, Some(v)) => Some(v),
            (Some(_), Some(_)) => {
                return Err(DuplicateKeyError {
                    path: leading_path.as_slice().into(),
                });
            }
            (None, None) => None,
        };

        lhs.children = lhs.children.merge(rhs.children, |pc, lhs, rhs| {
            Self::merge_root(leading_path.fork_append(pc), lhs, rhs)
        })?;

        Ok(lhs)
    }

    /// Merges the given child into this node at the given prefix.
    ///
    /// If there is already a child at the given prefix, the two children
    /// are recursively merged.
    fn merge_child(
        mut self: Box<Self>,
        mut leading_path: PathGuard<'_>,
        prefix: PathComponent,
        child: Box<Self>,
    ) -> Result<Box<Self>, DuplicateKeyError> {
        leading_path.extend(self.partial_path.components());
        leading_path.push(prefix);

        self.children[prefix] = Some(match self.children.take(prefix) {
            Some(existing) => Self::merge(leading_path, existing, child)?,
            None => child,
        });

        Ok(self)
    }

    /// Hashes this trie, returning a hashed trie.
    #[must_use]
    pub fn into_hashed_trie(self: Box<Self>) -> Box<HashedKeyValueTrieRoot<'a, T>> {
        HashedKeyValueTrieRoot::new(PathGuard::new(&mut PathBuf::new_const()), self)
    }
}

impl<'a, T: AsRef<[u8]> + ?Sized> HashedKeyValueTrieRoot<'a, T> {
    /// Constructs a new hashed key-value trie node from the given un-hashed
    /// node.
    #[must_use]
    pub fn new(
        mut leading_path: PathGuard<'_>,
        #[expect(clippy::boxed_local)] node: Box<KeyValueTrieRoot<'a, T>>,
    ) -> Box<Self> {
        let children = node
            .children
            .map(|pc, child| child.map(|child| Self::new(leading_path.fork_append(pc), child)));

        Box::new(Self {
            computed: HashableShunt::new(
                leading_path.as_slice(),
                node.partial_path,
                node.value.map(|v| ValueDigest::Value(v.as_ref())),
                children
                    .each_ref()
                    .map(|_, c| c.as_deref().map(|c| c.computed.clone())),
            )
            .to_hash(),
            leading_path: leading_path.as_slice().into(),
            partial_path: node.partial_path,
            value: node.value,
            children,
        })
    }
}

impl<T: AsRef<[u8]> + ?Sized> TrieNode<T> for KeyValueTrieRoot<'_, T> {
    type PartialPath<'a>
        = PackedPathRef<'a>
    where
        Self: 'a;

    fn partial_path(&self) -> Self::PartialPath<'_> {
        self.partial_path
    }

    fn value(&self) -> Option<&T> {
        self.value
    }

    fn child_hash(&self, pc: PathComponent) -> Option<&HashType> {
        let _ = pc;
        None
    }

    fn child_node(&self, pc: PathComponent) -> Option<&Self> {
        self.children[pc].as_deref()
    }

    fn child_state(&self, pc: PathComponent) -> Option<super::TrieEdgeState<'_, Self>> {
        self.children[pc]
            .as_deref()
            .map(|node| super::TrieEdgeState::UnhashedChild { node })
    }
}

impl<T: AsRef<[u8]> + ?Sized> TrieNode<T> for HashedKeyValueTrieRoot<'_, T> {
    type PartialPath<'a>
        = PackedPathRef<'a>
    where
        Self: 'a;

    fn partial_path(&self) -> Self::PartialPath<'_> {
        self.partial_path
    }

    fn value(&self) -> Option<&T> {
        self.value
    }

    fn child_hash(&self, pc: PathComponent) -> Option<&HashType> {
        self.children[pc].as_deref().map(|c| &c.computed)
    }

    fn child_node(&self, pc: PathComponent) -> Option<&Self> {
        self.children[pc].as_deref()
    }

    fn child_state(&self, pc: PathComponent) -> Option<super::TrieEdgeState<'_, Self>> {
        self.children[pc]
            .as_deref()
            .map(|node| super::TrieEdgeState::from_node(node, Some(&node.computed)))
    }
}

impl<T: AsRef<[u8]> + ?Sized> HashedTrieNode<T> for HashedKeyValueTrieRoot<'_, T> {
    fn computed(&self) -> &HashType {
        &self.computed
    }
}

impl<'a, T: AsRef<[u8]> + ?Sized> Hashable for HashedKeyValueTrieRoot<'a, T> {
    type LeadingPath<'b>
        = &'b [PathComponent]
    where
        Self: 'b;

    type PartialPath<'b>
        = PackedPathRef<'a>
    where
        Self: 'b;

    type FullPath<'b>
        = JoinedPath<&'b [PathComponent], PackedPathRef<'a>>
    where
        Self: 'b;

    fn parent_prefix_path(&self) -> Self::LeadingPath<'_> {
        &self.leading_path
    }

    fn partial_path(&self) -> Self::PartialPath<'_> {
        self.partial_path
    }

    fn full_path(&self) -> Self::FullPath<'_> {
        self.parent_prefix_path().append(self.partial_path)
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        self.value.map(|v| ValueDigest::Value(v.as_ref()))
    }

    fn children(&self) -> Children<Option<HashType>> {
        self.children
            .each_ref()
            .map(|_, c| c.as_deref().map(|c| c.computed.clone()))
    }
}

struct DebugValue<'a, T: ?Sized> {
    value: Option<&'a T>,
}

impl<'a, T: AsRef<[u8]> + ?Sized> DebugValue<'a, T> {
    const fn new(value: Option<&'a T>) -> Self {
        Self { value }
    }
}

impl<T: AsRef<[u8]> + ?Sized> std::fmt::Debug for DebugValue<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #![expect(clippy::indexing_slicing)]

        const MAX_BYTES: usize = 32;

        let Some(value) = self.value else {
            return write!(f, "None");
        };

        let value = value.as_ref();
        let truncated = &value[..value.len().min(MAX_BYTES)];

        let mut hex_buf = [0u8; MAX_BYTES * 2];
        let hex_buf = &mut hex_buf[..truncated.len().wrapping_mul(2)];
        hex::encode_to_slice(truncated, hex_buf).expect("exact fit");
        let s = str::from_utf8(hex_buf).expect("valid hex");

        if truncated.len() < value.len() {
            write!(f, "0x{s}... (len {})", value.len())
        } else {
            write!(f, "0x{s}")
        }
    }
}

struct DebugChildren<'a, T> {
    children: &'a Children<Option<T>>,
}

impl<'a, T: std::fmt::Debug> DebugChildren<'a, T> {
    const fn new(children: &'a Children<Option<T>>) -> Self {
        Self { children }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for DebugChildren<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            // if alternate, debug children as-is (which is pretty and recursive)
            self.children.fmt(f)
        } else {
            // otherwise, replace each child with a continuation marker
            self.children
                .each_ref()
                .map(
                    |_, child| {
                        if child.is_some() { "Some(...)" } else { "None" }
                    },
                )
                .fmt(f)
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use test_case::test_case;

    use super::*;

    /// in constant context, convert an ASCII hex string to a byte array
    ///
    /// # Panics
    ///
    /// Panics if the input is not valid hex.
    const fn from_ascii<const FROM: usize, const TO: usize>(hex: &[u8; FROM]) -> [u8; TO] {
        #![expect(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

        const fn from_hex_char(c: u8) -> u8 {
            match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => panic!("invalid hex character"),
            }
        }

        const {
            assert!(FROM == TO.wrapping_mul(2));
        }

        let mut bytes = [0u8; TO];
        let mut i = 0_usize;
        while i < TO {
            let off = i.wrapping_mul(2);
            let hi = hex[off];
            let off = off.wrapping_add(1);
            let lo = hex[off];
            bytes[i] = (from_hex_char(hi) << 4) | from_hex_char(lo);
            i += 1;
        }

        bytes
    }

    #[cfg(feature = "branch_factor_256")]
    const fn ascii_components<const FROM: usize, const TO: usize>(
        hex: &[u8; FROM],
    ) -> [PathComponent; TO] {
        #[expect(unsafe_code)]
        // SAFETY: `from_ascii` will generate a compiler error if `FROM != TO * 2`,
        // and `PathComponent` is `u8` newtype wrapper.
        unsafe {
            std::ptr::from_ref(&from_ascii::<FROM, TO>(hex))
                .cast::<[PathComponent; TO]>()
                .read()
        }
    }

    #[cfg(not(feature = "branch_factor_256"))]
    const fn ascii_components<const LEN: usize>(hex: &[u8; LEN]) -> [PathComponent; LEN] {
        #![expect(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

        const fn from_hex_char(c: u8) -> u8 {
            match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => panic!("invalid hex character"),
            }
        }

        let mut out = *hex;
        let mut i = 0_usize;
        while i < LEN {
            out[i] = from_hex_char(out[i]);
            i += 1;
        }

        #[expect(unsafe_code)]
        // SAFETY: we converted each byte to a value in 0..=15 above
        unsafe {
            std::ptr::from_ref(&out)
                .cast::<[PathComponent; LEN]>()
                .read()
        }
    }

    macro_rules! expected_hash {
        (
            merkledb16: $hex16:expr,
            merkledb256: $hex256:expr,
            ethereum: rlp($hexeth:expr),
        ) => {
            match () {
                #[cfg(all(not(feature = "branch_factor_256"), not(feature = "ethhash")))]
                () => $crate::HashType::from(from_ascii($hex16)),
                #[cfg(all(feature = "branch_factor_256", not(feature = "ethhash")))]
                () => $crate::HashType::from(from_ascii($hex256)),
                #[cfg(all(not(feature = "branch_factor_256"), feature = "ethhash"))]
                () => $crate::HashType::Rlp(smallvec::SmallVec::from(
                    &from_ascii::<{ $hexeth.len() }, { $hexeth.len() / 2 }>($hexeth)[..],
                )),
                #[cfg(all(feature = "branch_factor_256", feature = "ethhash"))]
                () => compile_error!("branch_factor_256 and ethhash cannot both be enabled"),
            }
        };
        (
            merkledb16: $hex16:expr,
            merkledb256: $hex256:expr,
            ethereum: $hexeth:expr,
        ) => {
            $crate::HashType::from(from_ascii(match () {
                #[cfg(all(not(feature = "branch_factor_256"), not(feature = "ethhash")))]
                () => $hex16,
                #[cfg(all(feature = "branch_factor_256", not(feature = "ethhash")))]
                () => $hex256,
                #[cfg(all(not(feature = "branch_factor_256"), feature = "ethhash"))]
                () => $hexeth,
                #[cfg(all(feature = "branch_factor_256", feature = "ethhash"))]
                () => compile_error!("branch_factor_256 and ethhash cannot both be enabled"),
            }))
        };
    }

    macro_rules! path {
        ($($pc:expr),* $(,)?) => {
            [$(const { $crate::PathComponent::ALL[$pc] },)*]
        };
    }

    macro_rules! cfg_alt {
        (
            $cfg:meta => $enabled:block else $disabled:block
        ) => {
            match () {
                #[cfg($cfg)]
                () => $enabled
                #[cfg(not($cfg))]
                () => $disabled
            }
        };
    }

    macro_rules! ascii_path {
        ($hex:expr) => {
            const {
                cfg_alt! {
                    feature = "branch_factor_256" => {
                        self::ascii_components::<{ $hex.len() }, { $hex.len() / 2 }>($hex)
                    } else {
                        self::ascii_components::<{ $hex.len() }>($hex)
                    }
                }
            }
        };
    }

    #[test_case(&[])]
    #[test_case(&[("a", "1")])]
    #[test_case(&[("a", "1"), ("b", "2")])]
    #[test_case(&[("a", "1"), ("ab", "2")])]
    #[test_case(&[("a", "1"), ("b", "2"), ("c", "3")])]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3")])]
    #[test_case(&[("a", "1"), ("b", "2"), ("ba", "3")])]
    #[test_case(&[("a", "1"), ("ab", "2"), ("abc", "3")])]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4")])]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4"), ("ba", "5")])]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4"), ("ba", "5"), ("bb", "6")])]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4"), ("ba", "5"), ("bb", "6"), ("c", "7")])]
    fn test_trie_from_slice(slice: &[(&str, &str)]) {
        let root = KeyValueTrieRoot::<str>::from_slice(slice).unwrap();
        if slice.is_empty() {
            if let Some(root) = root {
                panic!("expected None, got {root:#?}");
            }
        } else {
            let root = root.unwrap();
            eprintln!("trie: {root:#?}");
            for ((kvp_path, kvp_value), &(slice_key, slice_value)) in root
                .iter_values()
                .zip(slice)
                .chain(root.iter_values_desc().zip(slice.iter().rev()))
            {
                let slice_key = PackedPathRef::path_from_packed_bytes(slice_key.as_bytes());
                assert!(
                    kvp_path.path_eq(&slice_key),
                    "expected path {} got {}",
                    slice_key.display(),
                    kvp_path.display(),
                );
                assert_eq!(kvp_value, slice_value);
            }
        }
    }

    #[test]
    fn test_trie_from_slice_duplicate_keys() {
        let slice = [("a", "1"), ("ab", "2"), ("a", "3")];
        let err = KeyValueTrieRoot::<str>::from_slice(&slice).unwrap_err();
        assert_eq!(
            err,
            DuplicateKeyError {
                path: PathBuf::path_from_packed_bytes(b"a")
            }
        );
    }

    #[test]
    fn test_trie_from_unsorted_slice() {
        let slice = [("b", "2"), ("a", "1"), ("ab", "3")];
        let expected = [("a", "1"), ("ab", "3"), ("b", "2")];
        let root = KeyValueTrieRoot::<str>::from_slice(&slice)
            .unwrap()
            .unwrap();
        eprintln!("trie: {root:#?}");

        for ((kvp_path, kvp_value), &(slice_key, slice_value)) in root
            .iter_values()
            .zip(&expected)
            .chain(root.iter_values_desc().zip(expected.iter().rev()))
        {
            let slice_key = PackedPathRef::path_from_packed_bytes(slice_key.as_bytes());
            assert!(
                kvp_path.path_eq(&slice_key),
                "expected path {} got {}",
                slice_key.display(),
                kvp_path.display(),
            );
            assert_eq!(kvp_value, slice_value);
        }
    }

    #[test_case(&[("a", "1")], expected_hash!{
        merkledb16: b"1ffe11ce995a9c07021d6f8a8c5b1817e6375dd0ea27296b91a8d48db2858bc9",
        merkledb256: b"831a115e52af616bd2df8cd7a0993e21e544d7d201e151a7f61dcdd1a6bd557c",
        ethereum: rlp(b"c482206131"),
    }; "single key")]
    #[test_case(&[("a", "1"), ("b", "2")], expected_hash!{
        merkledb16: b"ff783ce73f7a5fa641991d76d626eefd7840a839590db4269e1e92359ae60593",
        merkledb256: b"301e9035ef0fe1b50788f9b5bca3a2c19bce9c798bdb1dda09fc71dd22564ce4",
        ethereum: rlp(b"d81696d580c22031c220328080808080808080808080808080"),
    }; "two disjoint keys")]
    #[test_case(&[("a", "1"), ("ab", "2")], expected_hash!{
        merkledb16: b"c5def8c64a2f3b8647283251732b68a2fb185f8bf92c0103f31d5ec69bb9a90c",
        merkledb256: b"2453f6f0b38fd36bcb66b145aff0f7ae3a6b96121fa1187d13afcffa7641b156",
        ethereum: rlp(b"d882006194d3808080808080c2323280808080808080808031"),
    }; "two nested keys")]
    #[test_case(&[("a", "1"), ("b", "2"), ("c", "3")], expected_hash!{
        merkledb16: b"95618fd79a0ca2d7612bf9fd60663b81f632c9a65e76bb5bc3ed5f3045cf1404",
        merkledb256: b"f5c185a96ed86da8da052a52f6c2e7368c90d342c272dd0e6c9e72c0071cdb0c",
        ethereum: rlp(b"da1698d780c22031c22032c2203380808080808080808080808080"),
    }; "three disjoint keys")]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3")], expected_hash!{
        merkledb16: b"ee8a7a1409935f58ab6ce40a1e05ee2a587bdc06c201dbec7006ee1192e71f70",
        merkledb256: b"40c9cee60ac59e7926109137fbaa5d68642d4770863b150f98bd8ac00aedbff3",
        ethereum: b"6ffab67bf7096a9608b312b9b2459c17ec9429286b283a3b3cdaa64860182699",
    }; "two children of same parent")]
    #[test_case(&[("a", "1"), ("b", "2"), ("ba", "3")], expected_hash!{
        merkledb16: b"d3efab83a1a4dd193c8ae51dfe638bba3494d8b1917e7a9185d20301ff1c528b",
        merkledb256: b"e6f711e762064ffcc7276e9c6149fc8f1050e009a21e436e7b78a4a60079e3ba",
        ethereum: b"21a118e1765c556e505a8752a0fd5bbb4ea78fb21077f8488d42862ebabf0130",
    }; "nested sibling")]
    #[test_case(&[("a", "1"), ("ab", "2"), ("abc", "3")], expected_hash!{
        merkledb16: b"af11454e2f920fb49041c9890c318455952d651b7d835f5731218dbc4bde4805",
        merkledb256: b"5dc43e88b3019050e741be52ed4afff621e1ac93cd2c68d37f82947d1d16cff5",
        ethereum: b"eabecb5e4efb9b5824cd926fac6350bdcb4a599508b16538afde303d72571169",
    }; "linear nested keys")]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4")], expected_hash!{
        merkledb16: b"749390713e51d3e4e50ba492a669c1644a6d9cb7e48b2a14d556e7f953da92fc",
        merkledb256: b"30dbf15b59c97d2997f4fbed1ae86d1eab8e7aa2dd84337029fe898f47aeb8e6",
        ethereum: b"2e636399fae96dc07abaf21167a34b8a5514d6594e777635987e319c76f28a75",
    }; "four keys")]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4"), ("ba", "5")], expected_hash!{
        merkledb16: b"1c043978de0cd65fe2e75a74eaa98878b753f4ec20f6fbbb7232a39f02e88c6f",
        merkledb256: b"02bb75b5d5b81ba4c64464a5e39547de4e0d858c04da4a4aae9e63fc8385279d",
        ethereum: b"df930bafb34edb6d758eb5f4dd9461fc259c8c13abf38da8a0f63f289e107ecd",
    }; "five keys")]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4"), ("ba", "5"), ("bb", "6")], expected_hash!{
        merkledb16: b"c2c13c095f7f07ce9ef92401f73951b4846a19e2b092b8a527fe96fa82f55cfd",
        merkledb256: b"56d69386ad494d6be42bbdd78b3ad00c07c12e631338767efa1539d6720ce7a6",
        ethereum: b"8ca7c3b09aa0a8877122d67fd795051bd1e6ff169932e3b7a1158ed3d66fbedf",
    }; "six keys")]
    #[test_case(&[("a", "1"), ("ab", "2"), ("ac", "3"), ("b", "4"), ("ba", "5"), ("bb", "6"), ("c", "7")], expected_hash!{
        merkledb16: b"697e767d6f4af8236090bc95131220c1c94cadba3e66e0a8011c9beef7b255a5",
        merkledb256: b"2f083246b86da1e6e135f771ae712f271c1162c23ebfaa16178ea57f0317bf06",
        ethereum: b"3fa832b90f7f1a053a48a4528d1e446cc679fbcf376d0ef8703748d64030e19d",
    }; "seven keys")]
    fn test_hashed_trie(slice: &[(&str, &str)], root_hash: crate::HashType) {
        let root = KeyValueTrieRoot::<str>::from_slice(slice)
            .unwrap()
            .unwrap()
            .into_hashed_trie();

        assert_eq!(*root.computed(), root_hash);
        assert_eq!(*root.computed(), crate::Preimage::to_hash(&*root));
    }

    macro_rules! long_key_pair {
        ($key:expr) => {
            (
                from_ascii::<{ $key.len() }, { $key.len() / 2 }>($key),
                #[expect(unsafe_code)]
                // SAFETY: from_ascii will produce a compiler error if $key is not valid ASCII
                unsafe {
                    std::str::from_utf8_unchecked($key)
                },
            )
        };
    }

    const LONG_KEY_SET: &[([u8; 32], &str)] = &[
        long_key_pair!(b"9e2df1e1a8198beda99bf3c002ca8347c6b523cce0fe4ca070fede084533ddda"), // depth = 3
        long_key_pair!(b"9e367d8b8d265c12f39b8a70fc71516e5bdc42ff2f76414498b3132ab1bf20d1"), // depth = 5
        long_key_pair!(b"9e36ecf376c2c57cbf812580866369ed3286dd83a4063ac145ec3c9e9de4b3a5"), // depth = 6
        long_key_pair!(b"9e36ef9105e33126cc3f42d52038cc6f611dbaae8235b5e1a10df86785915bac"), // depth = 6
        long_key_pair!(b"9e379f3b5f7f4bb72dd556dc0fe1dc5919a7f4dcd062f79693c530384acba92f"), // depth = 4
        long_key_pair!(b"9e77f62a25a9aa1836b06bb011a30f802094ef82e09325f15912a79d3e78cdde"), // depth = 3
        long_key_pair!(b"9e8ce4e24ca69d690c243aae6368d2693de30d71c2f05d51dd9b0407db2c05fe"), // depth = 3
        long_key_pair!(b"9e90247c3ee40b364e7ce377e7025acc866c7ac367d403c56e996a7225a9ab86"), // depth = 4
        long_key_pair!(b"9e9277cb39f210dfc44a61bde3d4945d7a98746d101c8eca1d77e3edf3cf705b"), // depth = 4
        long_key_pair!(b"9e9ae6b2be923b97ddaa422676faf5d7d5ed3161da3f597d5fcfb7a62977e2c8"), // depth = 4
        long_key_pair!(b"9e9d11c32010a3c62cf9623b30552fef9ed5ad8ca5bc5f6e5c67a007ea9b670b"), // depth = 4
        long_key_pair!(b"9ea24f32d07ee611e7b2f5a4d712ca013373281539a8666ea7ce610b9a7b97b9"), // depth = 3
        long_key_pair!(b"9ed6588c139b43b4563614d65f66fadd6571c11d451f3a8748696965b9dfb755"), // depth = 3
        long_key_pair!(b"9ee17f3fb0c2f08da973d1cd45c5c58e302f662926072ae363dce3fd9bb70510"), // depth = 3
        long_key_pair!(b"eb06927c08626368ef54ea9c9df813b42066bb6bd38e57cf3adcb1a73d151c96"), // depth = 3
        long_key_pair!(b"eb1420287f09faf6e7420a0076b3349bcb520d6c7071075b697d94b9c3c958bc"), // depth = 5
        long_key_pair!(b"eb14d141fd031dfa7cc2cdb7206916d0958eff25b33cb7c8b1c12704eafe6596"), // depth = 5
        long_key_pair!(b"eb1609842d46cffee7ed2aca354aa674154622235d21a5cdafbcac269450a27a"), // depth = 4
        long_key_pair!(b"eb21ca2f92bb3006fa04266aa5ce79523be99815046d5c085d2f9cd9d3d7cb02"), // depth = 3
        long_key_pair!(b"eb4172d683a2ae3ee91538a24ac3fdbef3120f88b3196a0dad11f74ffaa501f3"), // depth = 4
        long_key_pair!(b"eb4e17efc1badd021349d296a2f7b487a4876f2558f19ff705196331ff41c3cb"), // depth = 4
        long_key_pair!(b"eb5b4d35da26675cec092e942c5b1185f5b9e9dbf8f206b82f840f56642694ba"), // depth = 3
        long_key_pair!(b"eb71bdb74187a557754c15913fcac8b09d2aa584b920b48cd053c299c1f54d53"), // depth = 4
        long_key_pair!(b"eb77af42e8a3f6563da934ebe9717043efd4b0159ea3ab159da2fd6ac6c5c9bd"), // depth = 4
        long_key_pair!(b"eb83fc769ccf18bcdb3524fe4f6ecff858f42e557e45e1db09d46d9652f213ab"), // depth = 3
        long_key_pair!(b"ebaae8982c9baa21fbecf910e534c131bb8ef521ca31916f41c76db6e77e8610"), // depth = 3
        long_key_pair!(b"ebbde6c502af6a9d7b9528ca4771eeb12387a070d12339bcec4c4fbe03684e2f"), // depth = 3
        long_key_pair!(b"ebc14bc65b309689b407750345315f3462b02790f5b5502bd468f10a36c3ea4b"), // depth = 3
        long_key_pair!(b"ebd914688a5ccd5d6f7a5abb5403844b05481a61f86e0394a8d5c4ebbb3896b1"), // depth = 3
        long_key_pair!(b"ebe851e66cfdd6de7aa70e9a3388544cfbf52a488885c7a8f30b73a1ddee297e"), // depth = 3
        long_key_pair!(b"f90a98cb2c3634c6dafddb6342fb593d85fbe4c8e84eca1fdf3ef6a1c466d197"), // depth = 4
        long_key_pair!(b"f90a9ed5e87f13ddd6520b10625155a4494bc3c8ce4f1a4ce689bae4273b45a3"), // depth = 4
        long_key_pair!(b"f913f77bd5bdaeb1c25279835da3a9a6155bc5b2a250ca0a509fb236ee8f47bb"), // depth = 3
        long_key_pair!(b"f92ee120334ec8420c009c1720ff2a84be80928c68ce03b0537c5c239aa76a81"), // depth = 3
        long_key_pair!(b"f93924550802cb3ca89d00d99b0efc8d293a435212e4ab7d31e34d50208f2cb2"), // depth = 4
        long_key_pair!(b"f93e2d56543b0b66a1d6cfc246c9adf8d3fcde19cdd3ed9978cb1653417abde7"), // depth = 4
        long_key_pair!(b"f94269a17b5217e011bf1cf94c6647ed2e331628cd6ab2a2c4adb8e9e00f482d"), // depth = 4
        long_key_pair!(b"f9435a8fe5987742e08e24474278ccf689e0fe08cedab15c2fe30abf5b00ad94"), // depth = 5
        long_key_pair!(b"f9439c4a048f5dcfee0381c9a61e1d4a2a3e6e81e461ee1d9ba83d8c074cf919"), // depth = 5
        long_key_pair!(b"f94e6d76b6ef04457f79c9d1f7ea77533b1590b70974f1731ab6ab9b4d0c193e"), // depth = 4
        long_key_pair!(b"f95d29561ae7a697063d75cd321ad14a84ad27f57595e06ea9ab0fd277ba6cbb"), // depth = 4
        long_key_pair!(b"f95edeb78fb4bff225a042587ff1dba23cb7eb56f666c86722884c85e9a42616"), // depth = 4
        long_key_pair!(b"f9615793a5ca45058d25ba572cb38ea6ee886b70075b59bf24ce9631af3b0f0b"), // depth = 3
        long_key_pair!(b"f9759fbe1b46cdb0d657611b9be5404cc190ec1943bc15dd796cb10bfcd9cde7"), // depth = 3
        long_key_pair!(b"f98302fafb51fbcc620c71243a36596469844ecef76f9e54a03483d986c7309e"), // depth = 4
        long_key_pair!(b"f98710e3ed03c10a258f90c81e54d38bb64ba3fd6bb1ef900fbb06966cebb19f"), // depth = 4
        long_key_pair!(b"f992d982ccd6a9d10d4cb4afd1716015dd29e80d1972a6a5a4bdee7a530b4e67"), // depth = 3
        long_key_pair!(b"f9a614f03f2847ca35de1e2edd5f51ba9749e26196e829247bb3981a87d6c435"), // depth = 4
        long_key_pair!(b"f9adb4a95126734721f73d801416938ffe2a4b3c72d72b995465ebbfb939c592"), // depth = 4
        long_key_pair!(b"f9aeacacf382df181d159d999590426c0f461cbd61f821bcae37755280f3710c"), // depth = 4
        long_key_pair!(b"f9c222b1c4b822e2f170395fae0ba62137b49b23ee508973e1da2ad1b6046d43"), // depth = 4
        long_key_pair!(b"f9c2317e02eedb85dd790f119b9aef9041441829e14adad0803b88f2f26feb18"), // depth = 4
        long_key_pair!(b"f9c29f5cc179114beeb9bed23e34ca153a811d0498bf330ee3298a6720c3e9a5"), // depth = 4
        long_key_pair!(b"f9e188863067667acc89e4c5b952223c0011f88b583032273611c24b7e9fe3cc"), // depth = 3
        long_key_pair!(b"fa1138a84122dac254c467151f5f653b161c70018221952aa71c64d4615e7437"), // depth = 3
        long_key_pair!(b"fa225077c7e3305278560eeda2b28affc8fcfb50e7b5703a5b8ab228ef14c5c3"), // depth = 3
        long_key_pair!(b"fa3dc3442ccb6cc5e94d40178e54dcef15d9357fd88289781eaebe1239218fb5"), // depth = 3
        long_key_pair!(b"fa414ef4c7275836acc75243f51e4bcae0c2d300e5c04eddcb3d6c90b17e69d6"), // depth = 4
        long_key_pair!(b"fa440a6d6a070bf25ebae06125c7f962454d414a081e8346eec9e42fd851a488"), // depth = 5
        long_key_pair!(b"fa4485d725131d0b1e6fe516549f4c77764f9399645635f4eea0f35359be2ce8"), // depth = 5
        long_key_pair!(b"fa58df17031bab829a10bd52e2ab6f5228a45191a081130ae6283bcbeb1bba9d"), // depth = 3
        long_key_pair!(b"fa68816ea08f0eaf71f3fe5ee8cf531830c97e37069171d80e3add109b5391b8"), // depth = 3
        long_key_pair!(b"fa9a54da4b442e0de8e4750ed1c2efb3f0b453dfc126ca66e983e5abe0d86eea"), // depth = 3
        long_key_pair!(b"faadf92a924f7ee5b855cedf279998a56765b8994a4ddf4868f793d13470c980"), // depth = 4
        long_key_pair!(b"faaf320723637c35d96cac594f2c68993dee1cb876685de926b300324f615045"), // depth = 4
        long_key_pair!(b"fab36cb460c70adf7b94b668d7cefea73f7586d8f7addaa49ff57095c253ccd4"), // depth = 3
        long_key_pair!(b"fad28df88771f075a733204da373ec96a2619a1de28678dff7b0a9584667b1c8"), // depth = 3
        long_key_pair!(b"fd06d0334f0e127e2ba4982cf0d90d246997b2dcac20bdd3ff97beec06e8cb6b"), // depth = 4
        long_key_pair!(b"fd0e96768b07575f2b4c77d2e556cddfcfda408a1d7e560141fa4397918d26e0"), // depth = 4
        long_key_pair!(b"fd11ab24a03f0700219606aa8140d1147189a00db459a835a1d2ab1e40b1a60c"), // depth = 3
        long_key_pair!(b"fd242e9529f5f82b0f195320ee47ffef8bf321f630e456e8056b92e6256c6458"), // depth = 4
        long_key_pair!(b"fd2512ab6c89c7325e307d7dee5864a0e11d005d87f0e741ea96d7907f1cf49c"), // depth = 4
        long_key_pair!(b"fd2f35fc4a3bdf4410db224a3d50c65d850114cedd2f3c7ec77295ea1fa75d74"), // depth = 4
        long_key_pair!(b"fd3f4f56662a1c0ca63ebd031baa76ac19d0dd57993cd5565efd070fab0aeb06"), // depth = 3
        long_key_pair!(b"fd48cb88800879376d7015ac6757e5eb4b2c78630eda4ab5f371df4d3b88d1da"), // depth = 4
        long_key_pair!(b"fd4f53a0048add43827fb222adc145a49e3894e3fd0339b714857693dd4beedc"), // depth = 5
        long_key_pair!(b"fd4fa6f8d329df093d0d5f70862d4223f02bb1bad394021b6f57fad6bf621f2d"), // depth = 5
        long_key_pair!(b"fd5f5522f7dec0d9125fa051d651b15fb7c1804b0f1ce8dd3ed8257e15b5fe6a"), // depth = 3
        long_key_pair!(b"fd81510790c4c28ed405523efa9932e61e6f9380c53e0c7c22824c2f7595488d"), // depth = 4
        long_key_pair!(b"fd8503ab930fe253c9bf992a521a411c949a06cc1d6cecec2b870c7a9a5309ea"), // depth = 4
        long_key_pair!(b"fda7f08bb7e5bfbf86db4519f7fe0e7c2dec8e853288f2541be7d8f47820458e"), // depth = 4
        long_key_pair!(b"fdaca64ec8eb2a3f51e875f1c44cca3f22318a749b18e609e8ab988036d079cb"), // depth = 4
        long_key_pair!(b"fdda583b950fc10c2f96946bb1d949ceaa6fe718fa19c86d2ea99eeb8eaa9e13"), // depth = 3
        long_key_pair!(b"fde29b48b48655c051dd2289077385ee2ca617d7dd350fbe3742df38209a1195"), // depth = 4
        long_key_pair!(b"fdea9ef007c31c349a31df4aaeaa7ede0f0b732683f0f8b91dc241b55e75e17c"), // depth = 4
        long_key_pair!(b"fdef3f6b6f9bf08f0f1efe29f1718e5dc24bdb4812b12f7a58216326c9b38234"), // depth = 4
        long_key_pair!(b"fe0353c858f534524ede3ff8d3750167d6c9bdd34fee3f62b0e7d0c13928774c"), // depth = 4
        long_key_pair!(b"fe04306a4280ec088ffe79152371bd9b59818d30a38e4d7a4b1ebcc1dbbb152c"), // depth = 5
        long_key_pair!(b"fe04f6de6804017d6fda2084bdc57f2dd65283afc25c8e8b058f6931198e9508"), // depth = 5
        long_key_pair!(b"fe160187ba7863da61e125acde3980bdf8758b4086444f0d37415a7aeac990e1"), // depth = 3
        long_key_pair!(b"fe2988819bfef15e3e04a588cc956344faadb8f338196b15faf72b65fa503e1e"), // depth = 4
        long_key_pair!(b"fe2b9b759d7cd6a1692c2de0e3f856b8efddd597b18778cd0966e6912708c766"), // depth = 4
        long_key_pair!(b"fe79626c973f6d37e6f305a1c315452ee3315632ddc39add009835cdcebd7fbc"), // depth = 3
        long_key_pair!(b"fea484f01ce78325be9165b68b0acdf5969106ccf9f07f0cd6c76cdbf59daae9"), // depth = 4
        long_key_pair!(b"fea6525f4ca140405efe4617b68380f035455d7cdb6e0ba865ccd662c8a9f673"), // depth = 4
        long_key_pair!(b"fee788f6fae1bb133b0c60ae759a111664db6412db307afe0fad6096e67cb7e4"), // depth = 4
        long_key_pair!(b"feecd685e8775fa02103cbefe60f86a7aa345bc607645ea98811cfafd86ee18f"), // depth = 4
        long_key_pair!(b"fef73561e314d7dda4de5926ffc8b0e938cc97193c7b7011a17fd2ee591350bb"), // depth = 3
    ];

    struct IterPathTestCaseEdge<'a> {
        leading_path: &'a [PathComponent],
        node_path: &'a [PathComponent],
        node_value: Option<&'a str>,
        expected_children: &'a [PathComponent],
    }

    #[test_case(
        &[("a", "1")],
        "a",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path!(0x61),
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6, 1],
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            }
        };
        "single key, present"
    )]
    #[test_case(
        &[("a", "1")],
        "b",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path!(0x61),
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6, 1],
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            }
        };
        "single key, missing after"
    )]
    #[test_case(
        &[("a", "1")],
        "0",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path!(0x61),
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6, 1],
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            }
        };
        "single key, missing before"
    )]
    #[test_case(
        &[("a", "1"), ("b", "2")],
        "a",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x61, 0x62],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![0x61],
                        node_path: &path![],
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6],
                        node_value: None,
                        expected_children: &path![1, 2],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![6, 1],
                        node_path: &path![],
                        node_value: Some("1"),
                        expected_children: &path![],
                    },
                ]
            }
        };
        "two disjoint keys, first present"
    )]
    #[test_case(
        &[("a", "1"), ("b", "2")],
        "b",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x61, 0x62],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![0x62],
                        node_path: &path![],
                        node_value: Some("2"),
                        expected_children: &path![],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6],
                        node_value: None,
                        expected_children: &path![1, 2],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![6, 2],
                        node_path: &path![],
                        node_value: Some("2"),
                        expected_children: &path![],
                    },
                ]
            }
        };
        "two disjoint keys, second present"
    )]
    #[test_case(
        &[("a", "1"), ("b", "2")],
        "bad",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x61, 0x62],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![0x62],
                        node_path: &path![],
                        node_value: Some("2"),
                        expected_children: &path![],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6],
                        node_value: None,
                        expected_children: &path![1, 2],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![6, 2],
                        node_path: &path![],
                        node_value: Some("2"),
                        expected_children: &path![],
                    },
                ]
            }
        };
        "two disjoint keys, missing within"
    )]
    #[test_case(
        &[("a", "1"), ("b", "2")],
        "0",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x61, 0x62],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6],
                        node_value: None,
                        expected_children: &path![1, 2],
                    },
                ]
            }
        };
        "two disjoint keys, missing before"
    )]
    #[test_case(
        &[("a", "1"), ("b", "2")],
        "z",
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x61, 0x62],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![6],
                        node_value: None,
                        expected_children: &path![1, 2],
                    },
                ]
            }
        };
        "two disjoint keys, missing after"
    )]
    #[test_case(
        LONG_KEY_SET,
        from_ascii::<64, 32>(b"9e36ecf376c2c57cbf812580866369ed3286dd83a4063ac145ec3c9e9de4b3a5"),
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &ascii_path!(b""),
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x9e, 0xeb, 0xf9, 0xfa, 0xfd, 0xfe],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &ascii_path!(b"9e"),
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x2d, 0x36, 0x37, 0x77, 0x8c, 0x90, 0x92, 0x9a, 0x9d, 0xa2, 0xd6, 0xe1],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &ascii_path!(b"9e36"),
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x7d, 0xec, 0xef],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &ascii_path!(b"9e36ec"),
                        node_path: &ascii_path!(b"f376c2c57cbf812580866369ed3286dd83a4063ac145ec3c9e9de4b3a5"),
                        node_value: Some("9e36ecf376c2c57cbf812580866369ed3286dd83a4063ac145ec3c9e9de4b3a5"),
                        expected_children: &path![],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![9, 0xe, 0xf],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![9],
                        node_path: &path![0xe],
                        node_value: None,
                        expected_children: &path![2, 3, 7, 8, 9, 0xa, 0xd, 0xe],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![9, 0xe, 3],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![6, 7],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![9, 0xe, 3, 6],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![7, 0xe],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![9, 0xe, 3, 6, 0xe],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0xc, 0xf],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![9, 0xe, 3, 6, 0xe, 0xc],
                        node_path: &ascii_path!(b"f376c2c57cbf812580866369ed3286dd83a4063ac145ec3c9e9de4b3a5"),
                        node_value: Some("9e36ecf376c2c57cbf812580866369ed3286dd83a4063ac145ec3c9e9de4b3a5"),
                        expected_children: &path![],
                    },
                ]
            }
        };
        "many long keys, present"
    )]
    #[test_case(
        LONG_KEY_SET,
        [0_u8; 32],
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &ascii_path!(b""),
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x9e, 0xeb, 0xf9, 0xfa, 0xfd, 0xfe],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![9, 0xe, 0xf],
                    },
                ]
            }
        };
        "many long keys, missing zero key"
    )]
    #[test_case(
        LONG_KEY_SET,
        [0xff_u8; 32],
        cfg_alt! {
            feature = "branch_factor_256" => {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &ascii_path!(b""),
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![0x9e, 0xeb, 0xf9, 0xfa, 0xfd, 0xfe],
                    },
                ]
            } else {
                &[
                    IterPathTestCaseEdge {
                        leading_path: &path![],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![9, 0xe, 0xf],
                    },
                    IterPathTestCaseEdge {
                        leading_path: &path![0xf],
                        node_path: &path![],
                        node_value: None,
                        expected_children: &path![9, 0xa, 0xd, 0xe],
                    },
                ]
            }
        };
        "many long keys, missing ones key"
    )]
    fn test_iter_path(
        slice: &[(impl AsRef<[u8]>, &str)],
        target: impl AsRef<[u8]>,
        edges: &[IterPathTestCaseEdge<'_>],
    ) {
        let root = KeyValueTrieRoot::<str>::from_slice(slice).unwrap().unwrap();
        let target = PackedPathRef::path_from_packed_bytes(target.as_ref());

        let mut iter = root.iter_path(target);
        for &IterPathTestCaseEdge {
            leading_path,
            node_path,
            node_value,
            expected_children,
        } in edges
        {
            let (actual_path, actual_edge) = iter.next().expect("expected more edges");
            assert!(
                actual_path.path_eq(leading_path),
                "expected leading path {} got {}",
                leading_path.display(),
                actual_path.display(),
            );
            let node = actual_edge.node().expect("expected node");
            assert_eq!(
                node.value(),
                node_value,
                "expected node value {:?} got {:?} at path <{}:{}>",
                node_value,
                node.value(),
                actual_path.display(),
                node.partial_path.display()
            );
            assert!(
                node.partial_path.path_eq(node_path),
                "expected node path {} got {}",
                node_path.display(),
                node.partial_path.display(),
            );
            for (pc, child) in &node.children {
                if expected_children.contains(&pc) {
                    assert!(
                        child.is_some(),
                        "expected child at {pc:x} at path {}",
                        actual_path.display()
                    );
                } else {
                    assert!(
                        child.is_none(),
                        "expected no child at {pc:x} at path {}",
                        actual_path.display()
                    );
                }
            }
        }

        if let Some((actual_path, actual_edge)) = iter.next() {
            panic!(
                "expected no more edges, got edge at path {} with node {:?}",
                actual_path.display(),
                actual_edge.node()
            );
        }
    }
}

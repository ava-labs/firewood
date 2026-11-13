// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use arity_arrays::Arity16;
use arity_arrays::FixedArray;
use arity_arrays::PackedArray;

use crate::Child;
use crate::Node;
use crate::PathComponent;

const MAX_CHILDREN: usize = PathComponent::LEN;

/// Type alias for an iterator over the slots of a branch node's children
/// with its corresponding [`PathComponent`].
pub type ChildrenSlots<T> = std::iter::Zip<
    std::array::IntoIter<PathComponent, MAX_CHILDREN>,
    std::array::IntoIter<T, MAX_CHILDREN>,
>;

/// The type of iterator returned by [`Children::iter_present`].
pub type IterPresentRef<'a, T> = std::iter::FilterMap<
    ChildrenSlots<&'a Option<T>>,
    fn((PathComponent, &'a Option<T>)) -> Option<(PathComponent, &'a T)>,
>;

/// The type of iterator returned by [`Children::iter_present`].
pub type IterPresentMut<'a, T> = std::iter::FilterMap<
    ChildrenSlots<&'a mut Option<T>>,
    fn((PathComponent, &'a mut Option<T>)) -> Option<(PathComponent, &'a mut T)>,
>;

/// Type alias for a collection of children in a branch node.
///
/// Backed by [`arity_arrays::FixedArray`], indexed by [`PathComponent`]. Unlike
/// the previous inline-array representation this type is not `Copy` (the backing
/// array is heap-free but move-only); clone explicitly where a copy is needed.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Children<T>(FixedArray<T, Arity16>);

impl<T: std::fmt::Debug> std::fmt::Debug for Children<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.iter().map(|(pc, child)| (pc.as_u8(), child)))
            .finish()
    }
}

impl<T> Children<T> {
    /// Creates a new [`Children`] by calling `f` for each possible
    /// [`PathComponent`].
    #[must_use]
    pub fn from_fn(mut f: impl FnMut(PathComponent) -> T) -> Self {
        Self(FixedArray::from_fn(|index| f(PathComponent(index))))
    }

    /// Borrows each element and returns a [`Children`] wrapping the references.
    #[must_use]
    pub fn each_ref(&self) -> Children<&T> {
        Children(FixedArray::from_fn(|index| self.0.get(index)))
    }

    /// Borrows each element mutably and returns a [`Children`] wrapping the
    /// mutable references.
    #[must_use]
    pub fn each_mut(&mut self) -> Children<&mut T> {
        // `iter_mut` (via `Deref<[T]>`) hands out the 16 disjoint `&mut T` in
        // ascending slot order; `FixedArray::from_fn` fills slot i with the i-th,
        // so slot i ends up borrowing element i.
        let mut slots = self.0.iter_mut();
        Children(FixedArray::from_fn(|_index| {
            slots
                .next()
                .expect("FixedArray::iter_mut yields exactly MAX_CHILDREN items")
        }))
    }

    /// Returns a reference to the element at `index`.
    ///
    /// This is infallible because `index` is guaranteed to be in-bounds.
    #[must_use]
    pub fn get(&self, index: PathComponent) -> &T {
        self.0.get(index.0)
    }

    /// Returns a mutable reference to the element at `index`.
    ///
    /// This is infallible because `index` is guaranteed to be in-bounds.
    #[must_use]
    pub fn get_mut(&mut self, index: PathComponent) -> &mut T {
        self.0.get_mut(index.0)
    }

    /// Replaces the element at `index` with `value`, returning the previous
    /// value.
    ///
    /// This is infallible because `index` is guaranteed to be in-bounds.
    pub fn replace(&mut self, index: PathComponent, value: T) -> T {
        self.0.replace(index.0, value)
    }

    /// Maps each element to another value using `f`, returning a new
    /// [`Children`] containing the results.
    #[must_use]
    pub fn map<O>(self, mut f: impl FnMut(PathComponent, T) -> O) -> Children<O> {
        Children(self.0.map(|index, value| f(PathComponent(index), value)))
    }

    /// Returns an iterator over each element with its corresponding
    /// [`PathComponent`].
    pub fn iter(&self) -> ChildrenSlots<&T> {
        self.into_iter()
    }

    /// Returns a mutable iterator over each element with its corresponding
    /// [`PathComponent`].
    pub fn iter_mut(&mut self) -> ChildrenSlots<&mut T> {
        self.into_iter()
    }

    /// Merges this collection of children with another collection of children
    /// using the given function.
    ///
    /// If the function returns an error, the merge is aborted and the error is
    /// returned. Because this method takes `self` and `other` by value, they
    /// will be dropped if the merge fails.
    pub fn merge<U, V, E>(
        self,
        other: Children<U>,
        mut merge: impl FnMut(PathComponent, T, U) -> Result<Option<V>, E>,
    ) -> Result<Children<Option<V>>, E> {
        let mut output = Children::<Option<V>>::new();
        for ((pc, a), (_, b)) in self.into_iter().zip(other) {
            output[pc] = merge(pc, a, b)?;
        }
        Ok(output)
    }
}

impl<T> Children<Option<T>> {
    /// Creates a new [`Children`] with all elements set to `None`.
    #[must_use]
    pub fn new() -> Self {
        Self(FixedArray::new())
    }

    /// Returns the number of [`Some`] elements in this collection.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.count()
    }

    /// Sets the element at `index` to `None`, returning the previous value.
    #[must_use]
    pub fn take(&mut self, index: PathComponent) -> Option<T> {
        self.0.take(index.0)
    }

    /// Returns an iterator over each [`Some`] element with its corresponding
    /// [`PathComponent`].
    pub fn iter_present(&self) -> IterPresentRef<'_, T> {
        self.into_iter()
            .filter_map(|(pc, opt)| opt.as_ref().map(|v| (pc, v)))
    }

    /// If exactly one child is present, takes and returns it along with its
    /// index. Returns `None` if zero or more than one child is present.
    pub fn take_only_child(&mut self) -> Option<(PathComponent, T)> {
        let mut children = self.iter_present();

        // get first child, return None if not present
        let (idx, _) = children.next()?;
        if children.next().is_some() {
            // second child found
            return None;
        }
        drop(children);

        self.take(idx).map(|child| (idx, child))
    }
}

impl<T> Children<Option<T>> {
    /// Sets the child at `index` to `value`, returning the child it replaces.
    pub fn insert(&mut self, index: PathComponent, value: T) -> Option<T> {
        self.replace(index, Some(value))
    }

    /// Removes and returns the child at `index`.
    pub fn remove(&mut self, index: PathComponent) -> Option<T> {
        self.take(index)
    }

    /// Returns an iterator over each [`Some`] element, mutable in place, with
    /// its corresponding [`PathComponent`].
    pub fn iter_present_mut(&mut self) -> IterPresentMut<'_, T> {
        self.into_iter()
            .filter_map(|(pc, opt)| opt.as_mut().map(|v| (pc, v)))
    }
}

impl Children<Option<Child>> {
    /// Replaces the child at `index` with the result of `f`, which receives
    /// the current child by value.
    ///
    /// If `f` returns an error or panics, the child it consumed is gone and
    /// `index` is left absent. The caller must then drop the branch, not reuse
    /// or hash it. [`PackedChildren::try_update`] has the same contract.
    ///
    /// # Errors
    ///
    /// Returns the error `f` returns.
    pub fn try_update<E>(
        &mut self,
        index: PathComponent,
        f: impl FnOnce(Option<Child>) -> Result<Option<Child>, E>,
    ) -> Result<(), E> {
        let child = self.take(index);
        self[index] = f(child)?;
        Ok(())
    }
}

impl<T> Default for Children<Option<T>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T> Children<&'a Option<T>> {
    /// Returns the number of [`Some`] elements in this collection.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.iter().filter(|c| c.is_some()).count()
    }

    /// Returns an iterator over each [`Some`] element with its corresponding
    /// [`PathComponent`].
    pub fn iter_present(self) -> IterPresentRef<'a, T> {
        self.into_iter()
            .filter_map(|(pc, opt)| opt.as_ref().map(|v| (pc, v)))
    }
}

impl<'a, T> Children<&'a mut Option<T>> {
    /// Returns the number of [`Some`] elements in this collection.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.iter().filter(|c| c.is_some()).count()
    }

    /// Returns an iterator over each [`Some`] element with its corresponding
    /// [`PathComponent`].
    pub fn iter_present(self) -> IterPresentMut<'a, T> {
        self.into_iter()
            .filter_map(|(pc, opt)| opt.as_mut().map(|v| (pc, v)))
    }
}

impl<T> std::ops::Index<PathComponent> for Children<T> {
    type Output = T;

    fn index(&self, index: PathComponent) -> &Self::Output {
        self.get(index)
    }
}

impl<T> std::ops::IndexMut<PathComponent> for Children<T> {
    fn index_mut(&mut self, index: PathComponent) -> &mut Self::Output {
        self.get_mut(index)
    }
}

impl<T> IntoIterator for Children<T> {
    type Item = (PathComponent, T);
    type IntoIter = ChildrenSlots<T>;

    fn into_iter(self) -> Self::IntoIter {
        // Collect the owned values (ascending slot order) into an array so the
        // iterator type stays `Zip<array::IntoIter<PathComponent>, array::IntoIter<T>>`.
        let mut values = self.0.into_iter();
        let values: [T; MAX_CHILDREN] = std::array::from_fn(|_| {
            values
                .next()
                .expect("FixedArray yields exactly MAX_CHILDREN items")
                .1
        });
        PathComponent::ALL.into_iter().zip(values)
    }
}

impl<'a, T: 'a> IntoIterator for &'a Children<T> {
    type Item = (PathComponent, &'a T);
    type IntoIter = ChildrenSlots<&'a T>;

    fn into_iter(self) -> Self::IntoIter {
        let mut refs = self.0.iter();
        let refs: [&'a T; MAX_CHILDREN] =
            std::array::from_fn(|_| refs.next().expect("FixedArray has MAX_CHILDREN slots"));
        PathComponent::ALL.into_iter().zip(refs)
    }
}

impl<'a, T: 'a> IntoIterator for &'a mut Children<T> {
    type Item = (PathComponent, &'a mut T);
    type IntoIter = ChildrenSlots<&'a mut T>;

    fn into_iter(self) -> Self::IntoIter {
        let mut refs = self.0.iter_mut();
        let refs: [&'a mut T; MAX_CHILDREN] =
            std::array::from_fn(|_| refs.next().expect("FixedArray has MAX_CHILDREN slots"));
        PathComponent::ALL.into_iter().zip(refs)
    }
}

/// A pointer-sized, present-only container of a branch node's children, sized to
/// exactly the children that exist (zero heap when empty). Backed by
/// [`arity_arrays::PackedArray`], indexed by [`crate::U4`].
pub type DenseChildren<T> = PackedArray<T, Arity16>;

impl<T: Clone> Children<Option<T>> {
    /// Packs the present (`Some`) entries into a [`DenseChildren`], cloning each
    /// present value; empty slots are dropped (zero heap when empty).
    #[must_use]
    pub fn to_dense(&self) -> DenseChildren<T> {
        PackedArray::from(&self.0)
    }
}

impl<T: Clone> From<&DenseChildren<T>> for Children<Option<T>> {
    /// Expands a [`DenseChildren`] back into a full 16-slot [`Children`], cloning
    /// each present value; absent slots become `None`.
    fn from(dense: &DenseChildren<T>) -> Self {
        Children(FixedArray::from(dense))
    }
}

/// The children of a branch node that exist, indexed by [`PathComponent`].
///
/// One pointer wide. An empty collection owns no heap; otherwise it owns one
/// block sized to exactly the present children, in ascending index order.
/// Adding a child at an absent index, or removing a present one, reallocates
/// that block and moves the remaining children. Reading a child, mutating it
/// through [`get_mut`](Self::get_mut), and overwriting a present index are in
/// place.
///
/// [`DenseChildren`] names the same backing array indexed by [`crate::U4`].
#[derive(Clone, PartialEq, Eq)]
pub struct PackedChildren<T>(PackedArray<T, Arity16>);

impl<T> PackedChildren<T> {
    /// Creates an empty collection, which owns no heap.
    #[must_use]
    pub const fn new() -> Self {
        Self(PackedArray::new())
    }

    /// Returns the number of children.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.count()
    }

    /// Returns `true` if there are no children.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the child at `index`, if there is one.
    #[must_use]
    pub fn get(&self, index: PathComponent) -> Option<&T> {
        self.0.get(index.0)
    }

    /// Returns the child at `index` for mutation in place, if there is one.
    #[must_use]
    pub fn get_mut(&mut self, index: PathComponent) -> Option<&mut T> {
        self.0.get_mut(index.0)
    }

    /// Sets the child at `index` to `value`, returning the child it replaces.
    ///
    /// Reallocates only when `index` had no child.
    pub fn insert(&mut self, index: PathComponent, value: T) -> Option<T> {
        self.0.insert(index.0, value)
    }

    /// Removes and returns the child at `index`.
    ///
    /// Reallocates when `index` had a child.
    pub fn remove(&mut self, index: PathComponent) -> Option<T> {
        self.0.remove(index.0)
    }

    /// If exactly one child is present, removes and returns it along with its
    /// index. Returns `None`, changing nothing, if zero or more than one child
    /// is present.
    pub fn take_only_child(&mut self) -> Option<(PathComponent, T)> {
        self.0
            .take_only_child()
            .map(|(index, child)| (PathComponent(index), child))
    }

    /// Returns an iterator over each child with its [`PathComponent`], in
    /// ascending order.
    pub fn iter_present(&self) -> impl DoubleEndedIterator<Item = (PathComponent, &T)> {
        self.0
            .iter_present()
            .map(|(index, child)| (PathComponent(index), child))
    }

    /// Returns an iterator over each child, mutable in place, with its
    /// [`PathComponent`], in ascending order.
    pub fn iter_present_mut(&mut self) -> impl DoubleEndedIterator<Item = (PathComponent, &mut T)> {
        self.0
            .iter_present_mut()
            .map(|(index, child)| (PathComponent(index), child))
    }

    /// Returns the size in bytes of the heap block holding the children, or
    /// zero when empty. This is the size requested from the allocator, which
    /// may reserve more.
    #[must_use]
    pub fn allocated_size(&self) -> usize {
        self.0.allocated_size()
    }
}

impl<T> Default for PackedChildren<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for PackedChildren<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.iter_present().map(|(pc, child)| (pc.as_u8(), child)))
            .finish()
    }
}

impl<T> IntoIterator for PackedChildren<T> {
    type Item = (PathComponent, T);
    type IntoIter = std::iter::Map<
        <PackedArray<T, Arity16> as IntoIterator>::IntoIter,
        fn((crate::U4, T)) -> (PathComponent, T),
    >;

    /// Yields each child with its [`PathComponent`], in ascending order.
    fn into_iter(self) -> Self::IntoIter {
        self.0
            .into_iter()
            .map(|(index, child)| (PathComponent(index), child))
    }
}

impl<T> FromIterator<(PathComponent, T)> for PackedChildren<T> {
    /// Collects the children into one heap block. If an index repeats, the
    /// last value wins.
    fn from_iter<I: IntoIterator<Item = (PathComponent, T)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(index, child)| (index.0, child))
                .collect(),
        )
    }
}

impl<T> From<Children<Option<T>>> for PackedChildren<T> {
    /// Keeps the present (`Some`) slots, in one allocation.
    fn from(children: Children<Option<T>>) -> Self {
        Self(PackedArray::from(children.0))
    }
}

impl PackedChildren<Child> {
    /// Replaces the child at `index` with the result of `f`, which receives
    /// the current child by value.
    ///
    /// Reallocates only when `f` changes whether the child is present. If `f`
    /// returns an error or panics, the child it consumed is gone and `index`
    /// is left absent.
    ///
    /// After an error or unwind the branch no longer matches the trie it came
    /// from. The caller must drop it, not reuse or hash it.
    ///
    /// # Errors
    ///
    /// Returns the error `f` returns.
    pub fn try_update<E>(
        &mut self,
        index: PathComponent,
        f: impl FnOnce(Option<Child>) -> Result<Option<Child>, E>,
    ) -> Result<(), E> {
        let Some(slot) = self.get_mut(index) else {
            if let Some(child) = f(None)? {
                self.insert(index, child);
            }
            return Ok(());
        };

        // An empty leaf stands in for the child while `f` owns it, so the
        // block keeps its size. `guard` removes the stand-in unless a new
        // child is written over it.
        let child = std::mem::replace(slot, Child::Node(Node::default()));
        let mut guard = RemoveOnDrop {
            children: self,
            index,
            armed: true,
        };
        if let Some(new_child) = f(Some(child))?
            && let Some(slot) = guard.children.get_mut(index)
        {
            *slot = new_child;
            guard.armed = false;
        }
        Ok(())
    }
}

/// Removes the child at `index` when dropped, unless disarmed.
struct RemoveOnDrop<'a> {
    children: &'a mut PackedChildren<Child>,
    index: PathComponent,
    armed: bool,
}

impl Drop for RemoveOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.children.remove(self.index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LeafNode;
    use crate::Path;

    #[test]
    fn dense_children_round_trips_with_children() {
        // Children -> DenseChildren -> Children is the identity on present slots.
        let mut c: Children<Option<u32>> = Children::new();
        c[PathComponent::ALL[1]] = Some(10);
        c[PathComponent::ALL[9]] = Some(90);
        c[PathComponent::ALL[15]] = Some(150);

        let dense = c.to_dense();
        assert_eq!(dense.count(), 3);
        assert_eq!(dense.get(PathComponent::ALL[1].0), Some(&10));
        assert_eq!(dense.get(PathComponent::ALL[9].0), Some(&90));
        assert_eq!(dense.get(PathComponent::ALL[0].0), None);

        let back = Children::from(&dense);
        assert_eq!(back, c);

        // Empty round-trips with zero allocation (the #2099 property).
        let empty: Children<Option<u32>> = Children::new();
        let dense_empty = empty.to_dense();
        assert!(dense_empty.is_empty());
        assert_eq!(Children::from(&dense_empty), empty);
    }

    const fn pc(index: usize) -> PathComponent {
        PathComponent::ALL[index]
    }

    /// A new collection is empty, owns no heap block, and has no child at any index.
    #[test]
    fn packed_children_starts_empty() {
        let children = PackedChildren::<u32>::new();
        assert!(children.is_empty());
        assert_eq!(children.count(), 0);
        assert_eq!(children.allocated_size(), 0);
        for index in PathComponent::ALL {
            assert_eq!(children.get(index), None, "index {index:?} is present");
        }
    }

    /// Inserted values are readable at their index, an overwrite returns the
    /// previous value, and a removed index reads as absent.
    #[test]
    fn packed_children_insert_get_remove() {
        let entries = [(pc(9), 90), (pc(1), 10), (pc(15), 150)];
        let mut children = PackedChildren::new();
        for (index, value) in entries {
            assert_eq!(children.insert(index, value), None);
        }
        assert_eq!(children.count(), entries.len());
        for (index, value) in entries {
            assert_eq!(children.get(index), Some(&value));
        }

        let (index, value) = entries[0];
        assert_eq!(children.insert(index, value + 1), Some(value));
        *children.get_mut(index).expect("the index was just written") += 1;
        assert_eq!(children.get(index), Some(&(value + 2)));
        assert_eq!(
            children.count(),
            entries.len(),
            "an overwrite added a child"
        );

        assert_eq!(children.remove(index), Some(value + 2));
        assert_eq!(children.get(index), None);
        assert_eq!(children.remove(index), None);
        assert_eq!(children.count(), entries.len() - 1);
    }

    /// Every iterator yields the present children in ascending index order,
    /// whatever order they were inserted in.
    #[test]
    fn packed_children_iterates_in_ascending_order() {
        let inserted = [pc(12), pc(3), pc(7), pc(0)];
        let mut expected = inserted;
        expected.sort();

        let mut children: PackedChildren<u8> = inserted
            .into_iter()
            .map(|index| (index, index.as_u8()))
            .collect();

        let by_ref: Vec<_> = children.iter_present().map(|(index, _)| index).collect();
        assert_eq!(by_ref, expected);
        let reversed: Vec<_> = children
            .iter_present()
            .rev()
            .map(|(index, _)| index)
            .collect();
        assert!(reversed.iter().eq(expected.iter().rev()));
        let by_mut: Vec<_> = children
            .iter_present_mut()
            .map(|(index, _)| index)
            .collect();
        assert_eq!(by_mut, expected);
        let owned: Vec<_> = children.into_iter().collect();
        assert!(
            owned
                .into_iter()
                .eq(expected.map(|index| (index, index.as_u8())))
        );
    }

    /// `take_only_child` returns the child only when it is the sole one, and
    /// otherwise leaves the collection as it was.
    #[test]
    fn packed_children_take_only_child() {
        let mut children = PackedChildren::<u32>::new();
        assert_eq!(children.take_only_child(), None);

        children.insert(pc(4), 40);
        children.insert(pc(8), 80);
        assert_eq!(children.take_only_child(), None);
        assert_eq!(children.count(), 2, "a child was taken from a pair");

        children.remove(pc(8));
        assert_eq!(children.take_only_child(), Some((pc(4), 40)));
        assert!(children.is_empty());
    }

    /// Converting a dense `Children` keeps exactly its present slots, and
    /// collecting the same pairs builds an equal collection.
    #[test]
    fn packed_children_round_trips_through_children() {
        let entries = [(pc(2), 20), (pc(5), 50), (pc(14), 140)];
        let mut dense: Children<Option<u32>> = Children::new();
        for (index, value) in entries {
            dense[index] = Some(value);
        }

        let packed = PackedChildren::from(dense.clone());
        for (index, slot) in &dense {
            assert_eq!(packed.get(index), slot.as_ref(), "index {index:?} differs");
        }
        assert_eq!(packed, entries.into_iter().collect());
    }

    /// The heap block grows with every child and is released with the last one.
    #[test]
    fn packed_children_allocated_size_tracks_count() {
        let indices = [pc(1), pc(6), pc(11)];
        let mut children = PackedChildren::<u64>::new();
        let mut previous = children.allocated_size();
        for index in indices {
            children.insert(index, 0);
            let size = children.allocated_size();
            assert!(size > previous, "the block did not grow at {index:?}");
            previous = size;
        }
        for index in indices {
            children.remove(index);
        }
        assert_eq!(children.allocated_size(), 0);
    }

    fn leaf(value: u8) -> Child {
        Child::Node(Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: Box::from([value]),
        }))
    }

    /// A collection holding `leaf(1)` at `TARGET` and `leaf(2)` at `SIBLING`.
    fn target_and_sibling() -> PackedChildren<Child> {
        [(TARGET, leaf(1)), (SIBLING, leaf(2))]
            .into_iter()
            .collect()
    }

    const TARGET: PathComponent = pc(3);
    const SIBLING: PathComponent = pc(10);
    const ABSENT: PathComponent = pc(6);

    /// Replacing a present child hands the closure the old child, stores the
    /// new one, and reuses the heap block.
    #[test]
    fn try_update_present_to_present_is_in_place() {
        let mut children = target_and_sibling();
        let block = children.get(TARGET).map(std::ptr::from_ref);
        let size = children.allocated_size();

        let result: Result<(), ()> = children.try_update(TARGET, |child| {
            assert_eq!(child, Some(leaf(1)));
            Ok(Some(leaf(7)))
        });

        assert_eq!(result, Ok(()));
        assert_eq!(children.get(TARGET), Some(&leaf(7)));
        assert_eq!(children.get(SIBLING), Some(&leaf(2)));
        assert_eq!(
            children.get(TARGET).map(std::ptr::from_ref),
            block,
            "the child moved to a new block"
        );
        assert_eq!(children.allocated_size(), size);
    }

    /// A closure that returns no child removes a present one.
    #[test]
    fn try_update_present_to_absent_removes() {
        let mut children = target_and_sibling();

        let result: Result<(), ()> = children.try_update(TARGET, |_| Ok(None));

        assert_eq!(result, Ok(()));
        assert_eq!(children.get(TARGET), None);
        assert_eq!(children.get(SIBLING), Some(&leaf(2)));
        assert_eq!(children.count(), 1);
    }

    /// A failing closure on a present child propagates its error and leaves
    /// the index absent.
    #[test]
    fn try_update_present_error_leaves_absent() {
        let mut children = target_and_sibling();

        let result = children.try_update(TARGET, |_| Err("failed"));

        assert_eq!(result, Err("failed"));
        assert_eq!(children.get(TARGET), None);
        assert_eq!(children.get(SIBLING), Some(&leaf(2)));
        assert_eq!(children.count(), 1);
    }

    /// A panicking closure on a present child leaves the index absent and no
    /// placeholder child behind.
    #[test]
    fn try_update_present_panic_leaves_absent() {
        let mut children = target_and_sibling();

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), ()> = children.try_update(TARGET, |_| panic!("closure panicked"));
        }));

        assert!(unwound.is_err(), "the panic did not propagate");
        assert_eq!(children.get(TARGET), None);
        assert_eq!(children.get(SIBLING), Some(&leaf(2)));
        let placeholder = Child::Node(Node::default());
        assert!(
            children
                .iter_present()
                .all(|(_, child)| *child != placeholder),
            "a placeholder child is still in the collection"
        );
    }

    /// A closure that returns a child for an absent index inserts it.
    #[test]
    fn try_update_absent_to_present_inserts() {
        let mut children = target_and_sibling();

        let result: Result<(), ()> = children.try_update(ABSENT, |child| {
            assert_eq!(child, None);
            Ok(Some(leaf(9)))
        });

        assert_eq!(result, Ok(()));
        assert_eq!(children.get(ABSENT), Some(&leaf(9)));
        assert_eq!(children.get(TARGET), Some(&leaf(1)));
        assert_eq!(children.get(SIBLING), Some(&leaf(2)));
    }

    /// A closure that returns no child, or fails, for an absent index changes
    /// nothing.
    #[test]
    fn try_update_absent_stays_absent() {
        let mut children = target_and_sibling();
        let unchanged = children.clone();

        let none: Result<(), ()> = children.try_update(ABSENT, |_| Ok(None));
        assert_eq!(none, Ok(()));
        assert_eq!(children, unchanged);

        let failed = children.try_update(ABSENT, |_| Err("failed"));
        assert_eq!(failed, Err("failed"));
        assert_eq!(children, unchanged);
    }

    /// A dense collection holding `leaf(1)` at `TARGET` and `leaf(2)` at `SIBLING`.
    fn dense_target_and_sibling() -> Children<Option<Child>> {
        let mut children = Children::new();
        children.insert(TARGET, leaf(1));
        children.insert(SIBLING, leaf(2));
        children
    }

    /// On the dense collection, `insert` and `remove` return the child they
    /// displace, and `iter_present_mut` reaches exactly the present children.
    #[test]
    fn dense_insert_remove_and_iter_present_mut() {
        let mut children = dense_target_and_sibling();
        assert_eq!(children.insert(TARGET, leaf(7)), Some(leaf(1)));
        assert_eq!(children.insert(ABSENT, leaf(9)), None);

        let present: Vec<_> = children
            .iter_present_mut()
            .map(|(index, _)| index)
            .collect();
        let mut expected = [TARGET, SIBLING, ABSENT];
        expected.sort();
        assert_eq!(present, expected);

        assert_eq!(children.remove(TARGET), Some(leaf(7)));
        assert_eq!(children.remove(TARGET), None);
    }

    /// The dense `try_update` leaves each index present or absent exactly as
    /// the packed one does, for every combination of starting state and
    /// closure outcome.
    #[test]
    fn dense_try_update_matches_packed() {
        type Outcome = Result<Option<Child>, &'static str>;
        let outcomes: [fn() -> Outcome; 3] = [|| Ok(Some(leaf(7))), || Ok(None), || Err("failed")];

        for index in [TARGET, ABSENT] {
            for outcome in outcomes {
                let mut dense = dense_target_and_sibling();
                let mut packed = target_and_sibling();

                let dense_result = dense.try_update(index, |_| outcome());
                let packed_result = packed.try_update(index, |_| outcome());

                assert_eq!(dense_result, packed_result);
                assert_eq!(
                    PackedChildren::from(dense),
                    packed,
                    "the collections differ after updating {index:?} with {:?}",
                    outcome()
                );
            }
        }
    }
}

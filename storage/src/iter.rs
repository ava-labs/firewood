// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Internal utilities for working with iterators.

use std::num::NonZeroUsize;

/// Writes an iterator of displayable items to a writer, separated by a specified separator.
///
/// - If `limit` is `Some(n)`, it will write at most `n` items, followed by a message
///   indicating how many more items were not written.
/// - If `limit` is `None`, it will write all items without any limit.
///
/// # Arguments
/// - `writer`: The writer to which the items will be written.
/// - `iter`: An iterator of items that implement `std::fmt::Display`.
/// - `sep`: A separator that will be used between items.
/// - `limit`: An optional limit on the number of items to write.
///
/// # Returns
/// A `std::fmt::Result` indicating success or failure of the write operation.
pub(crate) fn write_iter(
    writer: &mut (impl std::fmt::Write + ?Sized),
    iter: impl IntoIterator<Item: std::fmt::Display>,
    sep: impl std::fmt::Display,
    limit: Option<NonZeroUsize>,
) -> std::fmt::Result {
    if let Some(limit) = limit {
        write_iter_with_limit(writer, iter, sep, limit)
    } else {
        write_iter_no_limit(writer, iter, sep)
    }
}

fn write_iter_no_limit(
    writer: &mut (impl std::fmt::Write + ?Sized),
    iter: impl IntoIterator<Item: std::fmt::Display>,
    sep: impl std::fmt::Display,
) -> std::fmt::Result {
    let mut iter = iter.into_iter();
    if let Some(item) = iter.next() {
        write!(writer, "{item}")?;
        for item in iter {
            write!(writer, "{sep}{item}")?;
        }
    }
    Ok(())
}

fn write_iter_with_limit(
    writer: &mut (impl std::fmt::Write + ?Sized),
    iter: impl IntoIterator<Item: std::fmt::Display>,
    sep: impl std::fmt::Display,
    limit: NonZeroUsize,
) -> std::fmt::Result {
    let mut iter = iter.into_iter();
    let Some(item) = iter.next() else {
        // Iterator is empty, nothing to write.
        return Ok(());
    };

    // Write the first item without a separator.
    write!(writer, "{item}")?;

    #[expect(clippy::arithmetic_side_effects, reason = "nonzero - 1 is safe")]
    let mut limit = limit.get() - 1; // -1 because we wrote the first item
    loop {
        match (iter.next(), limit.checked_sub(1)) {
            (Some(item), Some(new_limit)) => {
                // we can write another item, write it with the separator
                write!(writer, "{sep}{item}")?;
                limit = new_limit;
            }
            (Some(_), None) => {
                // we exhausted our limit, but there are still items left including the item
                // we just removed from the iterator.
                let n = iter.count().saturating_add(1); // +1 for the current item
                write!(writer, "{sep}... ({n} more hidden)")?;
                break;
            }
            (None, _) => break,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn test_write_iter_no_limit() {
        let mut output = String::new();
        let items = ["apple", "banana", "cherry"];
        write_iter(&mut output, &items, ", ", None).unwrap();
        assert_eq!(output, "apple, banana, cherry");
    }

    #[test]
    fn test_write_iter_with_limit() {
        let mut output = String::new();
        let items = ["apple", "banana", "cherry", "date"];
        write_iter(&mut output, &items, ", ", NonZeroUsize::new(2)).unwrap();
        assert_eq!(output, "apple, banana, ... (2 more hidden)");
    }
}

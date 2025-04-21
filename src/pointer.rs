/// An error occuring during offset calculation between pointers.
#[derive(Debug)]
pub enum OffsetCalculationError {
    /// The offset overflows an `isize`.
    OffsetDiffOverflowed,
    /// The size of the underlying pointer type does not fit in an `isize`.
    TypeDoesNotFitInIsize,
}

/// Calculates the offset between `from` and `to`, in increments of
/// `size_of::<T>()`.
///
/// # Errors
///
/// This function errors if:
/// - The offset overflows an `isize`.
/// - `size_of::<T>()` does not fit in an `isize`.
///
/// # Examples
///
/// ```
/// let lower: *const u8 = 0 as *const u8;
/// let higher: *const u8 = 3 as *const u8;
///
/// assert_eq!(offset_between(higher, lower), 3);
/// assert_eq!(offset_between(lower, higher), -3);
/// assert_eq!(offset_between(lower, lower), 0);
/// ```
pub fn offset_between<T: Sized>(
    to: *const T,
    from: *const T,
) -> Result<isize, OffsetCalculationError> {
    let to_offset = to as isize;
    let from_offset = from as isize;
    Ok(to_offset
        .checked_sub(from_offset)
        .ok_or(OffsetCalculationError::OffsetDiffOverflowed)?
        / isize::try_from(size_of::<T>())
            .map_err(|_| OffsetCalculationError::TypeDoesNotFitInIsize)?)
}

/// Indicates that a struct may be fallibly read from.
pub trait Readable<T> {
    /// Attempts to read from the struct, returning `None` if the read failed.
    fn read(&self) -> Option<T>;
}

/// Indicates that struct may be fallibly written to, returning `Err` if the
/// read failed.
pub trait Writable<T> {
    /// Attempts to write `v` to the struct.
    fn write(&self, v: T) -> Result<(), ()>;
}

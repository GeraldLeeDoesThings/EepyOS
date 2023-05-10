
pub trait Readable<T> {
    fn read(&self) -> Option<T>;
}

pub trait Writable<T> {
    fn write(&self, v: T) -> Result<(), ()>;
}


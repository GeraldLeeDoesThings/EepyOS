use core::{error::Error, fmt::Display, iter::Iterator};

pub trait Resource {
    fn exhausted(&self) -> bool;
}

#[derive(Debug)]
pub enum ResourceClaimError {
    AddedExhaustedResource,
    NoSpaceAvailable,
}

impl Display for ResourceClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AddedExhaustedResource => write!(f, "Attempted to add an exhausted resource."),
            Self::NoSpaceAvailable => write!(f, "Attempted to add a resource to a full manager."),
        }
    }
}

impl Error for ResourceClaimError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }

    fn description(&self) -> &'static str {
        "description() is deprecated; use Display"
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }

    fn provide<'a>(&'a self, _request: &mut core::error::Request<'a>) {}
}

pub struct ResourceManager<R: Resource, const SIZE: usize> {
    data: [R; SIZE],
}

impl<'a, R: Resource, const SIZE: usize> IntoIterator for &'a ResourceManager<R, SIZE> {
    type Item = &'a R;

    type IntoIter = impl Iterator<Item = Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, R: Resource, const SIZE: usize> IntoIterator for &'a mut ResourceManager<R, SIZE> {
    type Item = &'a mut R;

    type IntoIter = impl Iterator<Item = Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<R: Resource, const SIZE: usize> ResourceManager<R, SIZE> {
    pub const fn new(data: [R; SIZE]) -> Self {
        Self { data }
    }

    pub fn iter(&self) -> impl Iterator<Item = &R> {
        self.data.iter().filter(|r| !r.exhausted())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut R> {
        self.data.iter_mut().filter(|r| !r.exhausted())
    }

    pub fn claim_first(&mut self, new_resource: R) -> Result<usize, ResourceClaimError> {
        if new_resource.exhausted() {
            return Err(ResourceClaimError::AddedExhaustedResource);
        }

        match self
            .data
            .iter_mut()
            .enumerate()
            .find(|(_, resource)| resource.exhausted())
        {
            Some((index, resource)) => {
                *resource = new_resource;
                Ok(index)
            }
            None => Err(ResourceClaimError::NoSpaceAvailable),
        }
    }

    pub fn emplace_first(
        &mut self,
        make_new_resource: impl Fn(usize) -> R,
    ) -> Result<usize, ResourceClaimError> {
        match self
            .data
            .iter_mut()
            .enumerate()
            .find(|(_, resource)| resource.exhausted())
        {
            Some((index, resource)) => {
                *resource = make_new_resource(index);
                Ok(index)
            }
            None => Err(ResourceClaimError::NoSpaceAvailable),
        }
    }

    pub fn get_absolute(&self, index: usize) -> Option<&R> {
        self.data.get(index)
    }

    pub fn get_absolute_mut(&mut self, index: usize) -> Option<&mut R> {
        self.data.get_mut(index)
    }
}

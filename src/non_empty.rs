//! A collection guaranteed to contain at least one element

/// A collection guaranteed to contain at least one element
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NonEmpty<T>(Vec<T>);

impl<T> NonEmpty<T> {
    pub(crate) fn new(first: T) -> Self {
        Self(vec![first])
    }

    pub(crate) fn push(&mut self, item: T) {
        self.0.push(item);
    }

    pub(crate) fn first_mut(&mut self) -> &mut T {
        self.0.first_mut().expect("non-empty")
    }

    pub(crate) fn split_first(&self) -> (&T, &[T]) {
        self.0.split_first().expect("non-empty")
    }
}

impl<T> std::ops::Deref for NonEmpty<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

//! Owned individual types for the enum `std::path::Component`
//!
//! Useful for saying "the input to this function MUST be a normal component"

use std::{
    ffi::{OsStr, OsString},
    path::Component,
};

// Holds Exactly one component that is normal (not `.` or `..`)
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NormalComponent(OsString);

// Holds exactly one `.`
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurDirComponent;

// Holds exactly one `..`
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParentDirComponent;

// Holds root component, on windows root is generally prefix + root
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RootDirComponent;

// Holds prefix like `C:\`
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) struct PrefixComponent(OsString);

// Represents a single path part.
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum OwnedComponent {
    Normal(NormalComponent),
    CurDir(CurDirComponent),
    ParentDir(ParentDirComponent),
    #[allow(dead_code)]
    Prefix(PrefixComponent),
    RootDir(RootDirComponent),
}

impl AsRef<OsStr> for OwnedComponent {
    fn as_ref(&self) -> &OsStr {
        match self {
            OwnedComponent::Normal(inner) => inner.as_ref(),
            OwnedComponent::CurDir(inner) => inner.as_ref(),
            OwnedComponent::ParentDir(inner) => inner.as_ref(),
            OwnedComponent::Prefix(inner) => inner.as_ref(),
            OwnedComponent::RootDir(inner) => inner.as_ref(),
        }
    }
}

impl From<PrefixComponent> for OwnedComponent {
    fn from(value: PrefixComponent) -> Self {
        OwnedComponent::Prefix(value)
    }
}

impl From<RootDirComponent> for OwnedComponent {
    fn from(value: RootDirComponent) -> Self {
        OwnedComponent::RootDir(value)
    }
}

impl From<CurDirComponent> for OwnedComponent {
    fn from(value: CurDirComponent) -> Self {
        OwnedComponent::CurDir(value)
    }
}

impl From<ParentDirComponent> for OwnedComponent {
    fn from(value: ParentDirComponent) -> Self {
        OwnedComponent::ParentDir(value)
    }
}

impl From<NormalComponent> for OwnedComponent {
    fn from(value: NormalComponent) -> Self {
        OwnedComponent::Normal(value)
    }
}

pub(crate) fn owned(path: Component) -> OwnedComponent {
    match path {
        Component::Prefix(prefix_component) => {
            OwnedComponent::Prefix(PrefixComponent(prefix_component.as_os_str().to_os_string()))
        }
        Component::RootDir => OwnedComponent::RootDir(RootDirComponent),
        Component::CurDir => OwnedComponent::CurDir(CurDirComponent),
        Component::ParentDir => OwnedComponent::ParentDir(ParentDirComponent),
        Component::Normal(os_str) => OwnedComponent::Normal(NormalComponent(os_str.to_os_string())),
    }
}

impl AsRef<OsStr> for NormalComponent {
    fn as_ref(&self) -> &OsStr {
        self.0.as_ref()
    }
}

impl AsRef<OsStr> for ParentDirComponent {
    fn as_ref(&self) -> &OsStr {
        Component::ParentDir.as_ref()
    }
}

impl AsRef<OsStr> for CurDirComponent {
    fn as_ref(&self) -> &OsStr {
        Component::CurDir.as_ref()
    }
}

impl AsRef<OsStr> for RootDirComponent {
    fn as_ref(&self) -> &OsStr {
        Component::RootDir.as_ref()
    }
}

impl AsRef<OsStr> for PrefixComponent {
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

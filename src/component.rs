//! Owned individual types for the enum `std::path::Component`
//!
//! Useful for saying "the input to this function MUST be a normal component"

use std::{
    ffi::{OsStr, OsString},
    path::Component,
};

// Holds Exactly one component that is normal (not `.` or `..`)
#[derive(Debug)]
pub(crate) struct NormalComponent(OsString);

// Holds exactly one `.`
#[derive(Debug)]
pub(crate) struct CurDirComponent;

// Holds exactly one `..`
#[derive(Debug)]
pub(crate) struct ParentDirComponent;

// Holds root component, on windows root is generally prefix + root
#[derive(Debug)]
pub(crate) struct RootDirComponent;

// Holds prefix like `C:\`
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct PrefixComponent(OsString);

// Represents a single path part.
pub(crate) enum OwnedComponent {
    Normal(NormalComponent),
    CurDir(CurDirComponent),
    ParentDir(ParentDirComponent),
    #[allow(dead_code)]
    Prefix(PrefixComponent),
    RootDir(RootDirComponent),
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

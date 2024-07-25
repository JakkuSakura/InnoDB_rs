use std::{
    error::Error,
    fmt::{Debug, Display},
};

use page::PageType;

pub mod page;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InnoDBError {
    InvalidLength,
    InvalidChecksum,
    InvalidPageType { expected: PageType, has: PageType },
}

impl Display for InnoDBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl Error for InnoDBError {}
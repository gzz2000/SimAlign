//! Hierarchical name with index

use compact_str::CompactString;
use serde::{ Serialize, Deserialize };
use std::hash::{ Hash, Hasher };
use std::borrow::Borrow;
use std::fmt;
use itertools::Itertools;

#[derive(Hash, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HId(pub Vec<CompactString>, pub Option<i32>);
#[derive(Hash, Copy, Clone, PartialEq, Eq)]
pub struct RefHId<'a>(pub &'a [CompactString], pub Option<i32>);

impl fmt::Display for HId {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.iter().format("/"))?;
        if let Some(i) = self.1 {
            write!(f, "[{}]", i)?;
        }
        Ok(())
    }
}

impl fmt::Debug for HId {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

/// A general hier name with index, used as hashing.
pub trait HierNameIdx {
    fn hier(&self) -> &[CompactString];
    fn idx(&self) -> Option<i32>;
}

impl Hash for dyn HierNameIdx + '_ {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hier().hash(state);
        self.idx().hash(state);
    }
}

impl PartialEq for dyn HierNameIdx + '_ {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.idx() == other.idx() && self.hier() == other.hier()
    }
}

impl Eq for dyn HierNameIdx + '_ {}

impl HierNameIdx for HId {
    #[inline]
    fn hier(&self) -> &[CompactString] {
        &self.0
    }
    
    #[inline]
    fn idx(&self) -> Option<i32> {
        self.1
    }
}

impl<'i> Borrow<dyn HierNameIdx + 'i> for HId {
    #[inline]
    fn borrow(&self) -> &(dyn HierNameIdx + 'i) {
        self
    }
}

impl HierNameIdx for RefHId<'_> {
    #[inline]
    fn hier(&self) -> &[CompactString] {
        &self.0
    }
    
    #[inline]
    fn idx(&self) -> Option<i32> {
        self.1
    }
}

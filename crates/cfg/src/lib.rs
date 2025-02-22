//! cfg defines conditional compiling options, `cfg` attribute parser and evaluator

mod cfg_expr;
mod dnf;
#[cfg(test)]
mod tests;

use std::fmt;

use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use tt::SmolStr;

pub use cfg_expr::{CfgAtom, CfgExpr};
pub use dnf::DnfExpr;

/// Configuration options used for conditional compilation on items with `cfg` attributes.
/// We have two kind of options in different namespaces: atomic options like `unix`, and
/// key-value options like `target_arch="x86"`.
///
/// Note that for key-value options, one key can have multiple values (but not none).
/// `feature` is an example. We have both `feature="foo"` and `feature="bar"` if features
/// `foo` and `bar` are both enabled. And here, we store key-value options as a set of tuple
/// of key and value in `key_values`.
///
/// See: <https://doc.rust-lang.org/reference/conditional-compilation.html#set-configuration-options>
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct CfgOptions {
    enabled: FxHashSet<CfgAtom>,
}

impl CfgOptions {
    pub fn check(&self, cfg: &CfgExpr) -> Option<bool> {
        cfg.fold(&|atom| self.enabled.contains(atom))
    }

    pub fn insert_atom(&mut self, key: SmolStr) {
        self.enabled.insert(CfgAtom::Flag(key));
    }

    pub fn insert_key_value(&mut self, key: SmolStr, value: SmolStr) {
        self.enabled.insert(CfgAtom::KeyValue { key, value });
    }

    pub fn apply_diff(&mut self, diff: CfgDiff) {
        for atom in diff.enable {
            self.enabled.insert(atom);
        }

        for atom in diff.disable {
            self.enabled.remove(&atom);
        }
    }

    pub fn get_cfg_keys(&self) -> Vec<&SmolStr> {
        self.enabled
            .iter()
            .map(|x| match x {
                CfgAtom::Flag(key) => key,
                CfgAtom::KeyValue { key, .. } => key,
            })
            .collect()
    }

    pub fn get_cfg_values(&self, cfg_key: &str) -> Vec<&SmolStr> {
        self.enabled
            .iter()
            .filter_map(|x| match x {
                CfgAtom::KeyValue { key, value } if cfg_key == key => Some(value),
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CfgDiff {
    // Invariants: No duplicates, no atom that's both in `enable` and `disable`.
    enable: Vec<CfgAtom>,
    disable: Vec<CfgAtom>,
}

impl CfgDiff {
    /// Create a new CfgDiff. Will return None if the same item appears more than once in the set
    /// of both.
    pub fn new(enable: Vec<CfgAtom>, disable: Vec<CfgAtom>) -> Option<CfgDiff> {
        let mut occupied = FxHashSet::default();
        for item in enable.iter().chain(disable.iter()) {
            if !occupied.insert(item) {
                // was present
                return None;
            }
        }

        Some(CfgDiff { enable, disable })
    }

    /// Returns the total number of atoms changed by this diff.
    pub fn len(&self) -> usize {
        self.enable.len() + self.disable.len()
    }
}

impl fmt::Display for CfgDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.enable.is_empty() {
            f.write_str("enable ")?;
            for (i, atom) in self.enable.iter().enumerate() {
                let sep = match i {
                    0 => "",
                    _ if i == self.enable.len() - 1 => " and ",
                    _ => ", ",
                };
                f.write_str(sep)?;

                write!(f, "{}", atom)?;
            }

            if !self.disable.is_empty() {
                f.write_str("; ")?;
            }
        }

        if !self.disable.is_empty() {
            f.write_str("disable ")?;
            for (i, atom) in self.disable.iter().enumerate() {
                let sep = match i {
                    0 => "",
                    _ if i == self.enable.len() - 1 => " and ",
                    _ => ", ",
                };
                f.write_str(sep)?;

                write!(f, "{}", atom)?;
            }
        }

        Ok(())
    }
}

pub struct InactiveReason {
    enabled: Vec<CfgAtom>,
    disabled: Vec<CfgAtom>,
}

impl fmt::Display for InactiveReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.enabled.is_empty() {
            for (i, atom) in self.enabled.iter().enumerate() {
                let sep = match i {
                    0 => "",
                    _ if i == self.enabled.len() - 1 => " and ",
                    _ => ", ",
                };
                f.write_str(sep)?;

                write!(f, "{}", atom)?;
            }
            let is_are = if self.enabled.len() == 1 { "is" } else { "are" };
            write!(f, " {} enabled", is_are)?;

            if !self.disabled.is_empty() {
                f.write_str(" and ")?;
            }
        }

        if !self.disabled.is_empty() {
            for (i, atom) in self.disabled.iter().enumerate() {
                let sep = match i {
                    0 => "",
                    _ if i == self.disabled.len() - 1 => " and ",
                    _ => ", ",
                };
                f.write_str(sep)?;

                write!(f, "{}", atom)?;
            }
            let is_are = if self.disabled.len() == 1 { "is" } else { "are" };
            write!(f, " {} disabled", is_are)?;
        }

        Ok(())
    }
}

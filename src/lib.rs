//! ## `simalign`: simulation alignment
//!
//! This contains the core functionalities and data structures
//! for the behavior-based netlist alignment toolkit.
//!
//! See the binaries for example usage.

use indexmap::IndexMap;
use compact_str::CompactString;
use serde::{ Serialize, Deserialize };
use vcd_ng::{ Parser, FastFlow, ReferenceIndex, Var, ScopeItem, FastFlowToken, FFValueChange };
use std::fs::File;
use std::io::{ self, BufReader };
use std::hash::{ Hash, Hasher };
use std::borrow::Borrow;

/// A general hier name with index, used as hashing.
trait HierNameIdx {
    fn hier(&self) -> &[CompactString];
    fn idx(&self) -> Option<i32>;
}

impl Hash for dyn HierNameIdx + '_ {
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

impl HierNameIdx for (Vec<CompactString>, Option<i32>) {
    #[inline]
    fn hier(&self) -> &[CompactString] {
        &self.0
    }
    
    #[inline]
    fn idx(&self) -> Option<i32> {
        self.1
    }
}

impl<'i> Borrow<dyn HierNameIdx + 'i> for (Vec<CompactString>, Option<i32>) {
    #[inline]
    fn borrow(&self) -> &(dyn HierNameIdx + 'i) {
        self
    }
}

impl HierNameIdx for (&Vec<CompactString>, Option<i32>) {
    #[inline]
    fn hier(&self) -> &[CompactString] {
        &self.0
    }
    
    #[inline]
    fn idx(&self) -> Option<i32> {
        self.1
    }
}

/// The hash database.
#[derive(Serialize, Deserialize, Debug)]
pub struct HashDB {
    /// Net name to index in `hashes` vector.
    ///
    /// Vector bits will be mapped separately like
    /// (xxx, 0) -> (start, 1), regardless of the declaration.
    pub name2id: IndexMap<(Vec<CompactString>, Option<i32>), usize>,
    /// The flattened hash values.
    pub hashes: Vec<u64>,
}

/// The internal bit state
#[derive(Debug, Copy, Clone)]
struct BitState {
    /// The last switched period index (after).
    last_index: u64,
    /// The state of last valid switch.
    last_state: u8,
    /// The current state (valid switch candidate).
    cur_state: u8
}

impl BitState {
    /// update the hash if a switch happens.
    #[inline]
    fn update_hash(&self, h: &mut u64) {
        if self.last_index != 0 &&
            self.last_state != self.cur_state
        {
            // a switch happens on last_index.
            *h = h.wrapping_mul(80267270009u64)
                .wrapping_add(self.last_index)
                .wrapping_mul(257u64)
                .wrapping_add(self.cur_state as u64 + 1);
        }
    }
}

/// Recursively enumerate the var definitions in VCD header.
///
/// If a hier temp vector is provided, the callback function
/// will receive a hierarchy vector.
fn enumerate_vars(
    item: &[ScopeItem],
    mut hier: Option<&mut Vec<CompactString>>,
    f: &mut impl FnMut(&Var, Option<&Vec<CompactString>>)
) {
    for i in item {
        if let Some(h) = hier.as_mut() {
            if let Some(t) = match i {
                ScopeItem::Var(var) =>
                    Some(var.reference.as_str()),
                ScopeItem::Scope(scope) =>
                    Some(scope.identifier.as_str()),
                _ => None
            } {
                h.push(t.into());
            }
        }
        match i {
            ScopeItem::Var(var) => f(
                var, hier.as_ref().map(|r| &**r)),
            ScopeItem::Scope(scope) =>
                enumerate_vars(
                    &scope.children[..],
                    hier.as_mut().map(|r| &mut **r),
                    f
                ),
            _ => {}
        }
        if let Some(h) = hier.as_mut() {
            if let ScopeItem::Var(_) | ScopeItem::Scope(_) = i {
                h.pop();
            }
        }
    }
}

#[inline]
fn enumerate_bits(index: Option<ReferenceIndex>, f: &mut impl FnMut(Option<i32>, usize)) {
    use ReferenceIndex::*;
    match index {
        None => f(None, 0),
        Some(BitSelect(idx)) => f(Some(idx), 0),
        Some(Range(msb, lsb)) => {
            if msb > lsb {
                for (offset, i) in (lsb..(msb + 1)).rev().enumerate() {
                    f(Some(i), offset);
                }
            }
            else {
                for (offset, i) in (msb..(lsb + 1)).enumerate() {
                    f(Some(i), offset);
                }
            }
        }
    }
}

impl HashDB {
    /// Create a new empty hash database.
    #[inline]
    pub fn new() -> HashDB {
        HashDB {
            name2id: IndexMap::new(),
            hashes: Vec::new()
        }
    }

    /// Internal helper function:
    /// build metadata from a VCD file header, including a
    /// mapping between the IdCode and hash start positions.
    ///
    /// This will insert missing signals.
    /// If previous equaled signals are no longer equal,
    /// their hashes will be duplicated.
    #[inline]
    fn make_vcd_metadata(
        &mut self, vcd_file: &str
    ) -> io::Result<Vec<usize>> {
        // read the vcd header
        let f = File::open(vcd_file)?;
        let mut f = BufReader::with_capacity(65536, f);
        let mut parser = Parser::new(&mut f);
        let header = parser.parse_header()?;
        
        // then we build metadata (id to hash index).
        // we first calculate the number of idcodes.
        let old_num_names = self.name2id.len();
        let mut id_count = 0;
        enumerate_vars(&header.items[..], None, &mut |v, _| {
            id_count = (v.code.0 as usize + 1).max(id_count);
        });
        let mut id2hash = vec![usize::MAX; id_count];

        let mut hash_used = vec![u64::MAX; self.hashes.len()];
        // we enumerate all vars, and maintain these data:
        // 1. id2hash: we should obtain the mapping
        //    from idcode to hash start.
        // 2. self.name2id: we should insert any new (name, index)
        //    tuples there.
        // 3. hash_used: when alias is broken, we bails out.
        //
        // for multi-bit vectors, we have 1 entry in id2hash,
        // but multiple entries in name2id (should have indices
        // in the same order as multi-bit vcd signals)
        enumerate_vars(
            &header.items[..], Some(&mut Vec::new()),
            &mut |var, hier| {
                let hier = hier.unwrap();
                use ReferenceIndex::*;
                let first_bit = match var.index {
                    None => None,
                    Some(BitSelect(idx)) => Some(idx),
                    Some(Range(msb, _lsb)) => Some(msb)
                };

                let start = match self.name2id.get(&(
                    hier, first_bit
                ) as &dyn HierNameIdx) {
                    // name2id got the first entry.
                    Some(id) => *id,
                    // no entry, so we insert a new set of hashes.
                    None => {
                        let start = match id2hash[var.code.0 as usize] {
                            usize::MAX => {
                                let start = self.hashes.len();
                                self.hashes.extend((0..var.size).map(|_| 0));
                                hash_used.extend((0..var.size).map(|_| u64::MAX));
                                start
                            },
                            idp @ _ => idp
                        };
                        enumerate_bits(var.index, &mut |idx, offset| {
                            self.name2id.insert(
                                (hier.clone(), idx),
                                start + offset);
                        });
                        start
                    }
                };

                // check broken alias
                let hu = &mut hash_used[start];
                assert!(*hu == var.code.0 || *hu == u64::MAX);
                *hu = var.code.0;

                // store into hashes
                let idp = &mut id2hash[var.code.0 as usize];
                assert!(*idp == start || *idp == usize::MAX);
                *idp = start;
            }
        );
        if old_num_names != self.name2id.len() {
            if old_num_names == 0 {
                clilog::info!(
                    SIMAL_INIT,
                    "initialized db with {} names and {} hash bits",
                    self.name2id.len(), self.hashes.len()
                );
            }
            else {
                clilog::warn!(
                    SIMAL_REINIT,
                    "extended db with {} more names (total {})",
                    self.name2id.len() - old_num_names,
                    self.name2id.len()
                );
            }
        }
        Ok(id2hash)
    }

    /// Feed a VCD file to this database and update all
    /// hashes accordingly.
    pub fn feed_vcd(
        &mut self, vcd_file: &str,
        strobe_start: u64, strobe_period: u64
    ) -> io::Result<()> {
        // insert into hashes the separator between vcd files
        for v in self.hashes.iter_mut() {
            *v *= 100003;
        }
        // get hash indices
        let indices = self.make_vcd_metadata(vcd_file)?;
        // stream read signals and update hashes
        let f = File::open(vcd_file)?;
        let mut parser = FastFlow::new(f, 65536);
        let mut cur_time_id = 0;
        let mut states = vec![BitState {
            last_index: 0, last_state: 0, cur_state: 0
        }; self.hashes.len()];
        while let Some(tok) = parser.next_token()? {
            match tok {
                FastFlowToken::Timestamp(t) => {
                    // cur_ts = t;
                    cur_time_id = if t <= strobe_start { 0 }
                    else { (t - strobe_start) / strobe_period + 1 };
                },
                FastFlowToken::Value(FFValueChange{ id, bits }) => {
                    let id_st = indices[id.0 as usize];
                    for (i, &bit) in bits.iter().enumerate() {
                        let state = &mut states[id_st + i];
                        if state.last_index == cur_time_id {
                            state.cur_state = bit;
                        }
                        else {
                            state.update_hash(&mut self.hashes[id_st + i]);
                            state.last_index = cur_time_id;
                            state.last_state = state.cur_state;
                            state.cur_state = bit;
                        }
                    }
                }
            }
        }
        for (state, h) in states.iter().zip(
            self.hashes.iter_mut()
        ) {
            state.update_hash(h);
        }
        Ok(())
    }
}

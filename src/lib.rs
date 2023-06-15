//! ## `simalign`: simulation alignment
//!
//! This contains the core functionalities and data structures
//! for the behavior-based netlist alignment toolkit.
//!
//! See the binaries for example usage.

use indexmap::IndexMap;
use std::collections::HashMap;
use compact_str::CompactString;
use serde::{ Serialize, Deserialize };
use vcd_ng::{ Parser, FastFlow };
use std::fs::File;
use std::io::{ self, BufReader };

/// The hash database.
#[derive(Serialize, Deserialize, Debug)]
pub struct HashDB {
    /// Net name to start indices and widths into `hashes` vector.
    pub name2id: IndexMap<Vec<CompactString>, (usize, usize)>,
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
            *h = (
                *h * 80267270009u64 + self.last_index
            ) * 257u64 + self.cur_state as u64 + 1;
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
    /// mapping between the IdCode and hash positions.
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
        let old_num_names = self.name2id.len();
        use vcd_ng::{ ScopeItem, Var };
        fn enumerate_bits(
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
                        enumerate_bits(
                            &scope.children[..],
                            hier.as_mut().map(|r| &mut **r),
                            f
                        ),
                    _ => {}
                }
                if let Some(h) = hier.as_mut() {
                    if match i {
                        ScopeItem::Var(_) => true,
                        ScopeItem::Scope(_) => true,
                        _ => false
                    } {
                        h.pop();
                    }
                }
            }
        }
        let mut id_count = 0;
        enumerate_bits(&header.items[..], None, &mut |v, _| {
            id_count = (v.code.0 as usize + 1).max(id_count);
        });
        let mut id2hash = vec![usize::MAX; id_count];
        // used_hashes stores the hash places that were already
        // referenced. if two idcode refers to one hash place,
        // we need to do manual hash cloning.
        // start -> (idcode, width)
        let mut used_hashes =
            HashMap::<usize, (u64, usize)>::new();
        enumerate_bits(
            &header.items[..], Some(&mut Vec::new()),
            &mut |var, hier| {
                let hier = hier.unwrap();
                let (start, width) = match self.name2id.get(hier) {
                    Some(v) => *v,
                    None => {
                        let start = self.hashes.len();
                        self.hashes.extend((0..var.size).map(|_| 0));
                        used_hashes.insert(start, (
                            var.code.0, var.size as usize));
                        (start, var.size as usize)
                    }
                };
                assert_eq!(width, var.size as usize);
                let idp = &mut id2hash[var.code.0 as usize];
                assert!(*idp == start || *idp == usize::MAX);
                use std::collections::hash_map::Entry::*;
                match used_hashes.entry(start) {
                    Vacant(v) => {
                        *idp = start;
                        v.insert((var.code.0, width));
                    }
                    Occupied(o) if o.get().0 != var.code.0 => {
                        let owidth = o.get().1;
                        assert_eq!(owidth, width);
                        let nstart = self.hashes.len();
                        self.hashes.extend((0..width).map(|_| 0));
                        for i in 0..width {
                            self.hashes[nstart + i] =
                                self.hashes[start + i];
                        }
                        *idp = nstart;
                    }
                    _ => {
                        *idp = start;
                    }
                }
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
        use vcd_ng::{ FastFlowToken, FFValueChange };
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

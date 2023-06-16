//! Simulation database matching.
//!
//! This program reads in two databases with different signal
//! definitions (i.e., different netlist).
//! It outputs the signal pairs that are likely the same logic.

use simalign::{ HashDB, HId };
use ciborium::from_reader;
use std::fs::File;
use std::io::BufReader;
use indexmap::IndexMap;
use itertools::Itertools;

#[derive(clap::Parser, Debug)]
struct SimMatchArgs {
    /// The database 1
    db1: String,
    /// The database 2
    db2: String,
    /// The ignoring size threshold.
    ///
    /// If a matched group has size larger than this value,
    /// it will be ignored.
    #[clap(default_value_t = 30)]
    ignore_size: usize
}

fn main() {
    clilog::init_stderr_color_debug();
    let args = <SimMatchArgs as clap::Parser>::parse();
    println!("args: {:#?}", args);
    let db1: HashDB = from_reader(
        BufReader::new(File::open(args.db1).unwrap())
    ).unwrap();
    let db2: HashDB = from_reader(
        BufReader::new(File::open(args.db2).unwrap())
    ).unwrap();
    let mut pool = IndexMap::<u64, (Vec<&HId>, Vec<&HId>)>::new();
    macro_rules! enum_db {
        ($(($db:ident, $dbi:tt)),+) => ($(
            for (hid, p) in $db.name2id.iter() {
                use indexmap::map::Entry::*;
                let vs = match pool.entry($db.hashes[*p]) {
                    Occupied(o) => o.into_mut(),
                    Vacant(v) => v.insert(Default::default())
                };
                vs.$dbi.push(hid);
            }
        )+)
    }
    enum_db! {
        (db1, 0), (db2, 1)
    }
    println!("total bit types: {}", pool.len());
    println!("matched bit types: {}", pool.values()
             .filter(|(v1, v2)| v1.len() != 0 && v2.len() != 0)
             .count());
    println!("matched bit types (threshold {}): {}",
             args.ignore_size,
             pool.values()
             .filter(|(v1, v2)| v1.len() != 0 && v2.len() != 0
                     && v1.len() <= args.ignore_size
                     && v2.len() <= args.ignore_size)
             .count());
    // print all matched bit types..
    for (h, (v1, v2)) in pool.iter()
        .filter(|(_, (v1, v2))| v1.len() != 0 && v2.len() != 0
                && v1.len() <= args.ignore_size
                && v2.len() <= args.ignore_size)
    {
        println!("Hash {}: {{ {} }} = {{ {} }}",
                 h, v1.iter().format(", "), v2.iter().format(", "));
    }
}


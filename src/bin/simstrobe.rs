//! Simulation strobe processing.
//!
//! This program processes one input VCD file and analyses the
//! switching behavior. It outputs a database file containing the
//! hashes of switching activities.
//! 
//! It can optionally build on a previous database, which means
//! you can call it multiple times on multiple testbenches.
//!
//! The hash database is later used to align two netlists.

use simalign::HashDB;
use ciborium::{ from_reader, into_writer };
use std::fs::File;

#[derive(clap::Parser, Debug)]
struct SimStrobeArgs {
    /// The input vcd file path
    vcd: String,
    /// The strobe start timestamp (in VCD units)
    strobe_start: u64,
    /// The strobe period (in VCD units)
    strobe_period: u64,
    /// The database output file path.
    db_output: String,
    /// The optional previous database path.
    ///
    /// If not specified, a new one will be created.
    #[clap(long)]
    db_input: Option<String>,
}

fn main() {
    clilog::init_stderr_color_debug();
    let args = <SimStrobeArgs as clap::Parser>::parse();
    println!("args: {:?}", args);
    let mut db = match &args.db_input {
        Some(dbpath) => from_reader(
            File::open(dbpath).unwrap()
        ).unwrap(),
        None => HashDB::new()
    };
    db.feed_vcd(&args.vcd, args.strobe_start, args.strobe_period)
        .unwrap();
    into_writer(
        &db,
        File::create(&args.db_output).unwrap()
    ).unwrap();
}

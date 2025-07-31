#![feature(bufreader_peek)]
use actix::prelude::*;
use std::env::args;
use std::fs::File;
use std::io::BufReader;
use std::process::exit;

use env_logger::init;

mod bencoding;
mod metadata;

use crate::bencoding::Bencoding;
use crate::metadata::Metadata;

fn main() {
    init();

    let mut args = args();
    let metadata: Vec<String> = args.collect();
    if metadata.len() < 2 {
        eprintln!("Metadata file is required");

        return exit(1);
    }

    let metadata_file = &metadata[1];
    println!("Using metadata file {}", metadata_file);

    let handle = File::open(metadata_file).unwrap();
    let mut reader = BufReader::new(handle);

    let file = Bencoding::deserialize(&mut reader).unwrap();

    let metadata = Metadata::parse(file).unwrap();

    println!("Metadata: {:?}", metadata);

    return exit(0);
}

struct Download {}

struct Tracker {}

struct Peer {}

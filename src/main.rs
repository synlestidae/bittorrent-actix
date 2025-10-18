#![feature(bufreader_peek)]
use actix::prelude::*;
use std::env::args;
use std::fs::File;
use std::future::pending;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::process::exit;

use env_logger::init;
use log::{debug, error, info};
use sha1::{Digest, Sha1};

mod bittorrent;
mod commander;
mod coordinator;
mod filer;
mod init;
mod peer;
mod tracker;

use crate::bittorrent::{Bencoding, Metadata, Position};
use crate::filer::{PieceFile};
use crate::init::{Init, InitDownload};

#[actix::main(worker_threads = 50, max_blocking_threads = 50)]
async fn main() {
    println!("Starting up");
    init();

    let args = args();
    let metadata: Vec<String> = args.collect();
    if metadata.len() < 2 {
        eprintln!("Metadata file is required");

        exit(1);
    }

    let metadata_file = &metadata[1];

    // Open the file to decode metadata
    let handle = File::open(metadata_file).unwrap();
    let mut reader = BufReader::new(handle);
    let file = Bencoding::deserialize(&mut reader).unwrap();
    drop(reader);

    let metadata = Metadata::parse(file).unwrap();

    // Open the file again, this time for hashing
    let mut handle = File::open(metadata_file).unwrap();
    let info_hash = get_info_hash(&mut handle, metadata.info.position.clone());
    drop(handle);

    info!("Announce URL: {}", metadata.announce);
    info!("Pieces: {}", metadata.info.pieces.len());
    info!("Piece length: {}", metadata.info.pieces_length);
    info!(
        "Directory: {}",
        String::from_utf8_lossy(&metadata.info.name)
    );

    let info_hex = info_hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    info!("Info hash: {}", info_hex);

    debug!("Starting system");

    let filename = metadata.info.part_filename();
    let mut piece_file = PieceFile::open(&filename).await.unwrap();
    piece_file.initialise().await.unwrap();
    let init = Init::new(metadata, piece_file, info_hash).await;
    let init_addr: Addr<Init> = init.start();

    debug!("Sending InitDownload");
    if let Err(err) = init_addr.send(InitDownload {}).await {
        error!("Initialisation error: {}", err);
        exit(1)
    }

    pending::<()>().await;
}

fn get_info_hash(handle: &mut File, position: Position) -> [u8; 20] {
    let mut buffer = vec![0; position.length as usize];
    handle.seek(SeekFrom::Start(position.offset));
    handle.read_exact(&mut buffer);

    let mut hasher = Sha1::new();
    hasher.update(buffer);

    hasher.finalize().into()
}

struct Download {}

struct Tracker {}

struct Peer {}

#![feature(bufreader_peek)]
#![feature(string_from_utf8_lossy_owned)]
#![deny(unused_must_use)]
use actix::prelude::*;
use std::env::args;
use std::fs::File;
use std::future::pending;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::process::exit;

use env_logger::init;
use log::{debug, error};
use sha1::{Digest, Sha1};

mod bittorrent;
mod commander;
mod event;
mod filer;
mod init;
mod metadata_coordinator;
mod metadata_downloader;
mod peer;
mod peer_swarm;
mod piece_downloader;
mod piece_swarm;
mod piece_uploader;
mod tracker;

use crate::bittorrent::{Bencoding, Magnet, Metadata, Position, Uri};
use crate::event::EventHandler;
use crate::filer::PieceFile;
use crate::init::{Init, InitOptions, Ping};

#[actix::main(worker_threads = 50, max_blocking_threads = 50)]
async fn main() {
    println!("Starting up");
    env_logger::builder()
        .target(env_logger::Target::Stdout)
        .init();

    let args = args();
    let metadata: Vec<String> = args.collect();
    if metadata.len() < 2 {
        eprintln!("Metadata file is required");

        exit(1);
    }

    let event_handler = EventHandler::new().start();

    let metadata_file = &metadata[1];
    let init_addr: Addr<Init> = if metadata_file.starts_with("magnet:") {
        let uri = match Uri::parse(metadata_file) {
            Err(err) => {
                error!("Error parsing magnet uri: {:?}", err);
                exit(1);
            }
            Ok(u) => u,
        };
        let magnet = Magnet::parse(uri).unwrap();

        let options = InitOptions::Magnet(magnet.clone());

        let init = Init::with_metadata(magnet.info_hash, options, event_handler);

        init.start()
    } else {
        // Open the file to decode metadata
        let handle = File::open(metadata_file).unwrap();
        let mut reader = BufReader::new(handle);
        let file = Bencoding::deserialise(&mut reader).unwrap();
        drop(reader);

        let metadata = Metadata::parse(file).unwrap();

        // Open the file again, this time for hashing
        let mut handle = File::open(metadata_file).unwrap();
        let info_hash = get_info_hash(&mut handle, metadata.info.position.clone());
        drop(handle);

        let filename = metadata.info.part_filename();
        let mut piece_file = PieceFile::open(&filename).await.unwrap();
        piece_file.initialise().await.unwrap();

        let options = InitOptions::Metadata(metadata);
        let init = Init::with_metadata(info_hash, options, event_handler);

        init.start()
    };

    let fut = init_addr.send(Ping {});

    debug!("Starting system");

    pending::<()>().await;

    fut.await.expect("expected ping")
}

fn get_info_hash(handle: &mut File, position: Position) -> [u8; 20] {
    let mut buffer = vec![0; position.length as usize];
    handle
        .seek(SeekFrom::Start(position.offset))
        .expect("unable to read info hash");
    handle
        .read_exact(&mut buffer)
        .expect("unable to read info hash");

    let mut hasher = Sha1::new();
    hasher.update(buffer);

    hasher.finalize().into()
}

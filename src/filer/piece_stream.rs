use std::{pin::Pin, task::{Context, Poll}};
use actix::prelude::*;
use tokio::io::{AsyncRead, ReadBuf};

const BUFFER_SIZE: usize = 1 << 16;

// This is dumb, just use async code to read the pieces one by one, 
// and write them into the files one by one
//
// You have all the files and their lengths. When you've filled a file,
// get the next file. When there are no more files, you are done.
//
// If some files remain at completion, it is an error.
// If there is one piece remaining, it is an error.
// If there is one or more piece to analyse, it is an error.
// Otherwise, succes.
//
// If the file has an md5sum, and sum isn't correct, we can consider all of the pieces
// in that range to be corrupted, and drop them. This is a bit complicated though, some
// just report the bad file and bad pieces and move on. Mind you, it is a good way to check
// I wrote the code correctly.

pub struct PieceStream<T: AsyncRead> {
    byte_stream: T,
    buffer: [u8; BUFFER_SIZE],
}

impl<T: AsyncRead> PieceStream<T> {
    pub fn new(byte_stream: T) -> Self {
        Self {
            byte_stream,
            buffer: [0u8; BUFFER_SIZE],
        }
    }
}

pub struct SubPiece {
    pub piece_index: u32,
    pub offset: usize,
    pub data: Vec<u8>,
}

impl<T: AsyncRead> Stream for PieceStream<T> {
    type Item = SubPiece;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let byte_stream = Pin::new(&mut self.byte_stream).as_mut();
        let read_buf = ReadBuf::new(&mut self.buffer);
        byte_stream.poll_read(cx, &mut read_buf);
        unimplemented!()
    }
}

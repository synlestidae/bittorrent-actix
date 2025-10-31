use std::io::Result;

use crate::filer::PieceFile;
use std::cmp::min;

pub struct PieceReader {
    piece_file: PieceFile,
    next_piece_index: u32,
    current_piece_data: Vec<u8>,
    current_piece_offset: usize,
}

impl PieceReader {
    pub fn new(piece_file: PieceFile) -> Self {
        Self {
            piece_file,
            next_piece_index: 0,
            current_piece_offset: 0,
            current_piece_data: Vec::new(),
        }
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        let mut length = min(
            buf.len(),
            self.current_piece_data.len() - self.current_piece_offset
        );
        if length == 0 {
            self.current_piece_data = match self
                .piece_file
                .read_whole_piece(self.next_piece_index)
                .await?
            {
                Some(piece_data) => piece_data,
                None => return Ok(None),
            };
            self.next_piece_index += 1;
            self.current_piece_offset = 0;
            length = min(
                buf.len(),
                self.current_piece_data.len() - self.current_piece_offset
            );
        }

        let src = &self.current_piece_data[self.current_piece_offset..(self.current_piece_offset + length)];
        let dst = &mut buf[0..length];
        dst.copy_from_slice(src);

        self.current_piece_offset += length;

        Ok(Some(length))
    }
}

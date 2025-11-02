use std::io::Result;

use crate::filer::PieceFile;

pub struct PieceReader {
    piece_file: PieceFile,
    current_piece_index: u32,
    current_piece_offset: usize,
}

impl PieceReader {
    pub fn new(piece_file: PieceFile) -> Self {
        Self {
            piece_file,
            current_piece_index: 0,
            current_piece_offset: 0,
        }
    }

    pub async fn read(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        let start = self.current_piece_offset;
        let length = buf.len();

        let data = match self
            .piece_file
            .read(
                self.current_piece_index,
                self.current_piece_index,
                length as u32,
            )
            .await?
        {
            Some(d) => d,
            None => return Ok(None),
        };

        if data.len() < buf.len() {
            self.current_piece_index += 1;
            self.current_piece_offset = 0;
        } else {
            self.current_piece_offset += data.len();
        }

        let src = &data[start..(start + length)];
        let dst = &mut buf[0..length];
        dst.copy_from_slice(src);

        Ok(Some(length))
    }
}

use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::sync::Arc;

use log::{debug, trace};
use tokio::io::{self, AsyncBufReadExt};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::bittorrent::{BittorrentMessage, PIECE};
use crate::filer::FilePool;

pub const MAGIC_NUMBER: &'static [u8; 8] = b"PIECESv1";

#[derive(Clone)]
pub struct PieceFile {
    index: Arc<Mutex<BTreeMap<u32, u64>>>,
    file_pool: FilePool,
    highest_piece: i32,
}

const BITTORRENT_PIECE_MSG_ID: u8 = 7;

impl PieceFile {
    pub async fn open(path: &str) -> Result<Self, io::Error> {
        let file_pool = FilePool::open(path).await?;

        Ok(Self {
            file_pool,
            index: Arc::new(Mutex::new(BTreeMap::new())),
            highest_piece: -1,
        })
    }

    pub async fn initialise(&mut self) -> Result<(), io::Error> {
        let mut magic_buf = [0u8; 8];

        let mut writer = self.file_pool.writer().await?;
        let mut reader = BufReader::new(self.file_pool.reader().await?);

        reader.seek(SeekFrom::Start(0)).await?;

        let bytes_read = reader.read(&mut magic_buf).await?;
        if bytes_read == 0 {
            writer.write_all(MAGIC_NUMBER).await?;
        }
        trace!("Read {} bytes for magic number", bytes_read);
        trace!("Expected {:?} got {:?}", magic_buf, MAGIC_NUMBER);
        if magic_buf != *MAGIC_NUMBER {
            todo!("Need to handle this error");
        }

        let mut index = self.index.lock().await;
        while reader.fill_buf().await?.len() != 0 {
            let position = reader.stream_position().await?;

            let length = reader.read_u32().await?;
            trace!("Piece length is {}", length);
            let id = reader.read_u8().await?;
            if id != PIECE {
                // TODO this is a magic number
                todo!("Unexpected ID for piece: {}", id);
            }
            let piece_index = reader.read_u32().await?;

            debug!(
                "Adding existing piece {} at position {}",
                piece_index, position
            );
            if piece_index > self.highest_piece {
                self.highest_piece = piece_index as i32;
            }
            index.insert(piece_index, position);
            reader
                .seek(SeekFrom::Start(position + length as u64))
                .await?;
        }

        Ok(())
    }

    pub fn read(
        &mut self,
        piece_index: u32,
        offset: u32,
        length: u32,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, io::Error>> {
        let index_arc = self.index.clone();
        let mut file_pool = self.file_pool.clone();

        Box::pin(async move {
            let index = index_arc.lock().await;
            let position = match index.get(&piece_index) {
                Some(i) => i,
                None => return Ok(None),
            };
            let mut handle = file_pool.reader().await?;

            handle.seek(std::io::SeekFrom::Start(*position)).await?;

            let piece_length = handle.read_u32().await? - 9; // 9 = 1 + 2*4 = len + index + begin 
            if offset > piece_length {
                unimplemented!("Offset greater than length, should handle as error");
            }

            if handle.read_u8().await? != BITTORRENT_PIECE_MSG_ID {
                unimplemented!("Could not get ID, should handle as error");
            }

            if handle.read_u32().await? != piece_index {
                unimplemented!("Piece index doesn't match, should handle as error");
            }

            if handle.read_u32().await? != 0 {
                unimplemented!("Piece index wasn't zero, should handle as error");
            }

            let mut buf = vec![0u8; length as usize];
            handle
                .seek(std::io::SeekFrom::Current(offset as i64))
                .await?;
            handle.read_exact(&mut buf).await?;

            return Ok(Some(buf));
        })
    }

    /*pub fn read_whole_piece(
        &mut self,
        piece_index: u32,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, io::Error>> {
        let index_arc = self.index.clone();
        let mut file_pool = self.file_pool.clone();

        Box::pin(async move {
            let index = index_arc.lock().await;
            let position = match index.get(&piece_index) {
                Some(i) => i,
                None => return Ok(None),
            };
            let mut handle = file_pool.reader().await?;

            handle.seek(std::io::SeekFrom::Start(*position)).await?;

            let piece_length = handle.read_u32().await? - 9; // 9 = 1 + 2*4 = len + index + begin 
                                                            
            let mut buf = vec![0u8; piece_length as usize];
            handle
                .seek(std::io::SeekFrom::Current(5)) // 1 + 5 = id + len
                .await?;
            handle.read_exact(&mut buf).await?;

            return Ok(Some(buf));
        })
    }*/

    pub fn write(
        &mut self,
        piece_index: u32,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<(), io::Error>> {
        let piece = BittorrentMessage::Piece {
            index: piece_index,
            begin: 0,
            block: data,
        };
        let data = piece.serialize();
        let index_arc = self.index.clone();
        let mut file_pool = self.file_pool.clone();

        Box::pin(async move {
            let mut handle = file_pool.writer().await?;
            handle.seek(std::io::SeekFrom::End(0)).await?;
            let offset = handle.stream_position().await?;
            handle.write_all(&data).await?;

            let mut index = index_arc.lock().await;
            index.insert(piece_index, offset);

            Ok(())
        })
    }
}

#[cfg(test)]
mod test {
    use tokio;

    use crate::piece_file::PieceFile;

    #[tokio::test]
    pub async fn test_read_no_piece() {
        let mut piece_file = PieceFile::open("/tmp/piece_file.part").await.unwrap();

        assert_eq!(piece_file.read(0, 0, 4).await.unwrap(), None);
    }

    #[tokio::test]
    pub async fn test_write_read_piece() {
        let mut piece_file = PieceFile::open("/tmp/piece_file.part").await.unwrap();
        piece_file.write(0, vec![0, 1, 2, 3]).await.unwrap();

        assert_eq!(
            piece_file.read(0, 0, 4).await.unwrap(),
            Some(vec![0, 1, 2, 3])
        );
    }
}

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::io;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::filer::FilePool;
use crate::bittorrent::BittorrentMessage;

#[derive(Clone)]
pub struct PieceFile {
    index: Arc<Mutex<BTreeMap<u32, u64>>>,
    file_pool: FilePool,
}

const BITTORRENT_PIECE_MSG_ID: u8 = 7;

impl PieceFile {
    pub async fn open(path: &str) -> Result<Self, io::Error> {
        let file_pool = FilePool::open(path).await?;

        Ok(Self {
            file_pool,
            index: Arc::new(Mutex::new(BTreeMap::new())),
        })
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

            handle.seek(std::io::SeekFrom::Start(*position)).await;

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

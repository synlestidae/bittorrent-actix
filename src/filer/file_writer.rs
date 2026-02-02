use crate::bittorrent::{Info, Metadata, ModeFile};
use crate::filer::piece_reader::PieceReader;
use std::cmp::min;
use tokio::fs::OpenOptions;
use tokio::io::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

const BUFFER_SIZE: usize = 1 << 16;

struct File {
    directory: String,
    path: String,
    size: usize,
}

pub struct FileWriter {
    files: Vec<File>,
    src: PieceReader,
    piece_index: u32,
    offset: usize,
    info: Info,
}

#[derive(Debug)]
pub enum FileWriterError {
    Io { error: Error },
    PieceNotFound { piece_index: u32 },
    UnexpectedEndOfPieces,
    UnexpectedEndOfData,
}

impl From<Error> for FileWriterError {
    fn from(error: Error) -> Self {
        Self::Io { error }
    }
}

impl FileWriter {
    pub fn new(metadata: Metadata, src: PieceReader) -> Self {
        let files = match &metadata.info.file_info {
            ModeFile::Single { length } => {
                let path =
                    String::from_utf8_lossy(&metadata.info.name).replace(['/', '\\', '\0'], "_");
                vec![File {
                    directory: "./".to_owned(),
                    path,
                    size: *length as usize,
                }]
            }
            ModeFile::Multiple { files } => files
                .iter()
                .map(|file| {
                    let directory = file
                        .path
                        .iter()
                        .cloned()
                        .take(files.len() - 1)
                        .map(|s| String::from_utf8(s).unwrap())
                        .collect::<Vec<String>>()
                        .join("/");

                    let path = file
                        .path
                        .iter()
                        .cloned()
                        .map(|s| String::from_utf8(s).unwrap())
                        .collect::<Vec<String>>()
                        .join("/");

                    File {
                        directory,
                        path,
                        size: file.length as usize,
                    }
                })
                .collect(),
        };

        Self {
            files,
            src,
            piece_index: 0,
            offset: 0,
            info: metadata.info,
        }
    }

    pub async fn write_to_disk(&mut self) -> Result<(), FileWriterError> {
        const BUFFER_SIZE: usize = 1 << 16;
        let mut buffer = vec![0b0; BUFFER_SIZE];

        for file in self.files.iter() {
            let mut bytes_remaining = file.size;
            let mut file_out = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&file.path)
                .await?;

            while bytes_remaining > 0 {
                let length = min(bytes_remaining, buffer.len());
                let mut buf = &mut buffer[0..length];

                let bytes_read = match self.src.read(&mut buf).await? {
                    Some(b) => b,
                    None => return Err(FileWriterError::UnexpectedEndOfPieces),
                };
                if bytes_read == 0 {
                    return Err(FileWriterError::UnexpectedEndOfData);
                }

                file_out.write_all(&buf[0..bytes_read]).await?;

                bytes_remaining -= bytes_read;
            }
        }

        Ok(())
    }
}

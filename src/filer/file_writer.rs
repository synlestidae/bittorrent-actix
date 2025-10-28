use crate::bittorrent::{Info, Metadata, ModeFile};
use crate::filer::PieceFile;
use tokio::io::AsyncWriteExt;
use tokio::fs::{create_dir_all, File as FsFile};
use tokio::io::Error;
use std::cmp::min;

const BUFFER_SIZE: usize = 1 << 16;

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

struct File {
    directory: String,
    path: String,
    size: usize,
}

pub struct FileWriter {
    files: Vec<File>,
    piece_file: PieceFile,
    piece_index: u32,
    offset: usize,
    info: Info,
}

pub enum FileWriterError {
    Io { error: Error },
    PieceNotFound { piece_index: u32 },
}

impl From<Error> for FileWriterError {
    fn from(error: Error) -> Self {
        Self::Io { error }
    }
}

impl FileWriter {
    pub fn new(metadata: Metadata, piece_file: PieceFile) -> Self {
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

        Self { files, piece_file, piece_index: 0, offset: 0, info: metadata.info }
    }

    async fn write_to_disk(&mut self) -> Result<(), FileWriterError> {
        let mut piece_index = 0;
        let mut offset: u32 = 0;

        for file in self.files.iter() {
            let file_size = file.size as u32;
            let mut piece_length = self.info.piece_length(piece_index);

            create_dir_all(&file.directory).await?;

            let mut writer = FsFile::options()
                .read(false)
                .write(true)
                .create(true)
                .open(&file.path)
                .await?;

            let mut bytes_written = 0;
            while bytes_written < file_size {
                if offset >= piece_length {
                    piece_index += 1;
                    offset = 0;
                    piece_length = self.info.piece_length(piece_index);
                }
                let length = min(file_size - bytes_written, piece_length - offset);

                let block = self.piece_file.read(piece_index, offset as u32, length).await?.ok_or(FileWriterError::PieceNotFound { piece_index })?;
                writer.write_all(&block).await?;

                bytes_written += length;
                offset += length;
            }

        }

        Ok(())
    }
}

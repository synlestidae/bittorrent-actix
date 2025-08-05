use crate::bencoding::{Bencoding, Position};
use log::error;

#[derive(Debug)]
pub struct Metadata {
    pub announce: String,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub info: Info,
    pub creation_date: Option<u64>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub encoding: Option<String>,
}

#[derive(Debug)]
pub struct Info {
    pub pieces_length: u32,
    pub pieces: Vec<Vec<u8>>,
    pub file_info: ModeFile,
    pub name: Vec<u8>,
    pub position: Position,
}

#[derive(Debug)]
pub enum ModeFile {
    Single { length: i64 },
    Multiple { files: Vec<FileInfo> },
}

#[derive(Debug)]
pub struct FileInfo {
    pub length: i64,
    pub md5sum: Option<Vec<u8>>,
    pub path: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub enum MetadataError {
    UnexpectedType,
    MissingAnnounce,
    MissingInfo,
    MissingPieces,
    MissingPiecesLength,
    InvalidFileInfo,
    InvalidPath,
}

impl Metadata {
    pub fn parse(file: Bencoding) -> Result<Metadata, MetadataError> {
        let dictionary = match file {
            Bencoding::Dictionary { dictionary, .. } => dictionary,
            _ => return Err(MetadataError::UnexpectedType),
        };

        let info = dictionary
            .get(b"info".as_ref())
            .map(|info| Info::parse(info.clone()))
            .ok_or(MetadataError::MissingInfo)??;

        let announce = dictionary
            .get(b"announce".as_ref())
            .and_then(|v| match v {
                Bencoding::ByteString { byte_string, .. } => {
                    Some(String::from_utf8_lossy(byte_string).to_string())
                }
                _ => None,
            })
            .ok_or(MetadataError::MissingAnnounce)?;

        Ok(Metadata {
            info,
            announce,
            announce_list: None,
            creation_date: None,
            comment: None,
            created_by: None,
            encoding: None,
        })
    }

    pub fn total_file_bytes(&self) -> u64 {
        ((match &self.info.file_info {
            ModeFile::Single { length } => *length,
            ModeFile::Multiple { files } => files.iter().map(|f| f.length).sum(),
        }) as u64)
    }
}

impl Info {
    pub fn parse(info: Bencoding) -> Result<Info, MetadataError> {
        let (dictionary, position) = match info {
            Bencoding::Dictionary {
                dictionary,
                position: Some(position),
            } => (dictionary, position),
            _ => return Err(MetadataError::UnexpectedType),
        };

        let pieces_length = dictionary
            .get(b"piece length".as_ref())
            .and_then(|v| match v {
                Bencoding::Integer { integer, .. } => Some(integer),
                _ => None,
            })
            .ok_or(MetadataError::MissingPiecesLength)?;

        let pieces = dictionary
            .get(b"pieces".as_ref())
            .and_then(|v| match v {
                Bencoding::ByteString { byte_string, .. } => Some(parse_pieces(byte_string)),
                _ => None,
            })
            .ok_or(MetadataError::MissingPieces)??;

        let name = dictionary
            .get(b"name".as_ref())
            .and_then(|v| match v {
                Bencoding::ByteString { byte_string, .. } => Some(byte_string),
                _ => None,
            })
            .ok_or(MetadataError::MissingPiecesLength)?;

        if let Some(Bencoding::Integer {
            integer: length, ..
        }) = dictionary.get("length".as_bytes())
        {
            return Ok(Info {
                name: name.clone(),
                pieces_length: *pieces_length as u32,
                pieces,
                file_info: ModeFile::Single { length: *length },
                position,
            });
        }

        let files = match dictionary.get("files".as_bytes()) {
            Some(Bencoding::List { list, .. }) => list
                .iter()
                .map(parse_file)
                .collect::<Result<Vec<FileInfo>, MetadataError>>()?,
            _ => unimplemented!(),
        };

        Ok(Info {
            name: name.clone(),
            pieces_length: *pieces_length as u32,
            pieces,
            file_info: ModeFile::Multiple { files },
            position,
        })
    }
}

fn parse_file(element: &Bencoding) -> Result<FileInfo, MetadataError> {
    let dictionary = match element {
        Bencoding::Dictionary { dictionary, .. } => dictionary,
        _ => return Err(MetadataError::InvalidFileInfo),
    };

    match (
        dictionary.get("length".as_bytes()),
        dictionary.get("md5sum".as_bytes()),
        dictionary.get("path".as_bytes()),
    ) {
        (
            Some(Bencoding::Integer {
                integer: length, ..
            }),
            md5sum,
            Some(Bencoding::List { list: path, .. }),
        ) => {
            let md5sum = match md5sum {
                Some(Bencoding::ByteString {
                    byte_string: md5sum,
                    ..
                }) => Some(md5sum.clone()),
                None => None,
                _ => return Err(MetadataError::InvalidFileInfo),
            };

            let path: Vec<Vec<u8>> = path.iter().map(parse_path).collect::<Result<_, _>>()?;

            Ok(FileInfo {
                length: *length,
                md5sum,
                path: path.clone(),
            })
        }
        elements => {
            error!("Missing length, md5sum, and or path: {:?}", elements);

            Err(MetadataError::InvalidFileInfo)
        }
    }
}

fn parse_pieces(element: &Vec<u8>) -> Result<Vec<Vec<u8>>, MetadataError> {
    Ok(element.chunks(20).map(|chunk| chunk.to_vec()).collect())
}

fn parse_path(element: &Bencoding) -> Result<Vec<u8>, MetadataError> {
    match element {
        Bencoding::ByteString { byte_string, .. } => Ok(byte_string.clone()),
        _ => Err(MetadataError::InvalidPath),
    }
}

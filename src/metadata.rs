use crate::bencoding::Bencoding;
use log::{error, trace};

#[derive(Debug)]
pub struct Metadata {
    pub info: Info,
    pub announce: String,
    pub announce_list: Option<Vec<Vec<String>>>,
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
    pub path: Vec<u8>,
}

#[derive(Debug)]
pub enum MetadataError {
    UnexpectedType,
    MissingAnnounce,
    MissingInfo,
    MissingPieces,
    MissingPiecesLength,
    InvalidFileInfo,
}

impl Metadata {
    pub fn parse(file: Bencoding) -> Result<Metadata, MetadataError> {
        let dictionary = match file {
            Bencoding::Dictionary(dict) => dict,
            _ => return Err(MetadataError::UnexpectedType),
        };

        let info = dictionary
            .get(b"info".as_ref())
            .map(|info| Info::parse(info.clone()))
            .ok_or(MetadataError::MissingInfo)??;

        let announce = dictionary
            .get(b"announce".as_ref())
            .and_then(|v| match v {
                Bencoding::ByteString(bs) => Some(String::from_utf8_lossy(&bs).to_string()),
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
}

impl Info {
    pub fn parse(info: Bencoding) -> Result<Info, MetadataError> {
        let dictionary = match info {
            Bencoding::Dictionary(d) => d,
            _ => return Err(MetadataError::UnexpectedType),
        };

        let pieces_length = dictionary
            .get(b"piece length".as_ref())
            .and_then(|v| match v {
                Bencoding::Integer(pieces_length) => Some(pieces_length),
                _ => None,
            })
            .ok_or(MetadataError::MissingPiecesLength)?;

        println!("piece length {}", pieces_length);

        let pieces = dictionary
            .get(b"pieces".as_ref())
            .and_then(|v| match v {
                Bencoding::ByteString(pieces) => Some(parse_pieces(pieces)),
                _ => None,
            })
            .ok_or(MetadataError::MissingPieces)??;

        let name = dictionary
            .get(b"name".as_ref())
            .and_then(|v| match v {
                Bencoding::ByteString(n) => Some(n),
                _ => None,
            })
            .ok_or(MetadataError::MissingPiecesLength)?;

        if let Some(Bencoding::Integer(length)) = dictionary.get("length".as_bytes()) {
            return Ok(Info {
                name: name.clone(),
                pieces_length: *pieces_length as u32,
                pieces,
                file_info: ModeFile::Single { length: *length },
            });
        }

        let files = match dictionary.get("files".as_bytes()) {
            Some(Bencoding::List(files)) => files
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
        })
    }
}

fn parse_file(element: &Bencoding) -> Result<FileInfo, MetadataError> {
    let dictionary = match element {
        Bencoding::Dictionary(dictionary) => dictionary,
        _ => return Err(MetadataError::InvalidFileInfo),
    };

    match (
        dictionary.get("length".as_bytes()),
        dictionary.get("md5sum".as_bytes()),
        dictionary.get("path".as_bytes()),
    ) {
        (Some(Bencoding::Integer(length)), md5sum, Some(Bencoding::List(paths))) => {
            let md5sum = match md5sum {
                Some(Bencoding::ByteString(md5sum)) => Some(md5sum.clone()),
                None => None,
                _ => return Err(MetadataError::InvalidFileInfo),
            };

            Ok(FileInfo {
                length: *length,
                md5sum: md5sum,
                path: path.clone(),
            })
        }
        elements => {
            error!("Missing length, md5sum, and or path: {:?}", elements);

            return Err(MetadataError::InvalidFileInfo);
        }
    }
}

fn parse_pieces(element: &Vec<u8>) -> Result<Vec<Vec<u8>>, MetadataError> {
    Ok(element.chunks(20).map(|chunk| chunk.to_vec()).collect())
}

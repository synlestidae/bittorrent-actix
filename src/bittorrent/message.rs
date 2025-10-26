use std::error;
use std::fmt;

use super::ids::*;
use crate::bittorrent::NOT_INTERESTED;

#[derive(Debug)]
pub enum BittorrentMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have {
        piece_index: u32,
    },
    Bitfield {
        bitfield: Vec<u8>,
    },
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Port {
        port: u16,
    },
}

impl BittorrentMessage {
    pub fn deserialize(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < 4 {
            return Err(ParseError::InvalidLength);
        }
        let length = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;

        if length == 0 {
            return Ok(BittorrentMessage::KeepAlive);
        }

        if data.len() < 4 + length {
            return Err(ParseError::InvalidLength);
        }

        let id = data[4];
        let payload = &data[5..4 + length];

        match id {
            CHOKE => Ok(BittorrentMessage::Choke),
            UNCHOKE => Ok(BittorrentMessage::Unchoke),
            INTERESTED => Ok(BittorrentMessage::Interested),
            NOT_INTERESTED => Ok(BittorrentMessage::NotInterested),
            HAVE => {
                if payload.len() != 4 {
                    return Err(ParseError::InvalidLength);
                }
                let piece_index =
                    u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(BittorrentMessage::Have { piece_index })
            }
            BITFIELD => Ok(BittorrentMessage::Bitfield {
                bitfield: payload.to_vec(),
            }),
            REQUEST => {
                if payload.len() != 12 {
                    return Err(ParseError::InvalidLength);
                }
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let length = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);

                Ok(BittorrentMessage::Request {
                    index,
                    begin,
                    length,
                })
            }
            PIECE => {
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let block = payload[8..payload.len()].to_vec();

                Ok(BittorrentMessage::Piece {
                    index,
                    begin,
                    block,
                })
            }
            CANCEL => {
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let length = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);

                Ok(BittorrentMessage::Cancel {
                    index,
                    begin,
                    length,
                })
            }
            PORT => {
                let port = u16::from_be_bytes([payload[0], payload[1]]);

                Ok(BittorrentMessage::Port { port })
            }
            id => Err(ParseError::UnknownId(id)),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        match self {
            BittorrentMessage::KeepAlive => vec![0, 0, 0, 0],
            BittorrentMessage::Choke => vec![0, 0, 0, 1, 0],
            BittorrentMessage::Unchoke => vec![0, 0, 0, 1, 1],
            BittorrentMessage::Interested => vec![0, 0, 0, 1, 2],
            BittorrentMessage::NotInterested => vec![0, 0, 0, 1, 3],
            BittorrentMessage::Have { piece_index } => {
                let mut v: Vec<u8> = Vec::with_capacity(4 + 1 + 4);
                v.extend_from_slice(&(1u32 + 4u32).to_be_bytes());
                v.push(4);
                v.extend_from_slice(&piece_index.to_be_bytes());
                v
            }
            BittorrentMessage::Bitfield { bitfield } => {
                let mut v = Vec::with_capacity(4 + 1 + bitfield.len());
                v.extend_from_slice(&(1 + bitfield.len() as u32).to_be_bytes());
                v.push(5);
                v.extend_from_slice(bitfield);
                v
            }
            BittorrentMessage::Request {
                index,
                begin,
                length,
            } => {
                let mut v = Vec::with_capacity(4 + 1 + 12);
                v.extend_from_slice(&(1u32 + 12u32).to_be_bytes());
                v.push(6);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(&length.to_be_bytes());
                v
            }
            BittorrentMessage::Piece {
                index,
                begin,
                block,
            } => {
                let mut v = Vec::with_capacity(4 + 1 + 2 * (4) + block.len());
                v.extend_from_slice(&(9u32 + block.len() as u32).to_be_bytes());
                v.push(7);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(block);
                v
            }
            BittorrentMessage::Cancel {
                index,
                begin,
                length,
            } => {
                let mut v = Vec::with_capacity(4 + 1 + 3 * 4);
                v.extend_from_slice(&(3u32 * 4u32).to_be_bytes());
                v.push(8);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(&length.to_be_bytes());
                v
            }
            BittorrentMessage::Port { port } => {
                let mut v = Vec::with_capacity(4 + 1 + 2);
                v.extend_from_slice(&(1u32 + 2u32).to_be_bytes());
                v.push(9);
                v.extend_from_slice(&port.to_be_bytes());
                v
            }
        }
    }
}

#[derive(Debug)]
pub enum ParseError {
    InvalidLength,
    UnknownId(u8),
    Malformed,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidLength => write!(f, "invalid length"),
            ParseError::UnknownId(_) => write!(f, "unknown id"),
            ParseError::Malformed => write!(f, "malformed"),
        }
    }
}

impl error::Error for ParseError {}

pub struct BittorrentHandshake {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl BittorrentHandshake {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(49 + 20 + 20);
        buf.push(19);
        buf.extend_from_slice(b"BitTorrent protocol");
        buf.extend_from_slice(&[0; 8]);
        buf.extend_from_slice(&self.info_hash);
        buf.extend_from_slice(&self.peer_id);
        buf
    }

    pub fn len() -> usize {
        49 + 20 + 20
    }
}

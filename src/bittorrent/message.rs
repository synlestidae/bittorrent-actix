use std::collections::HashMap;
use std::error;
use std::fmt;
use std::io::BufReader;
use std::io::Cursor;

use super::ids::*;
use crate::bittorrent::Bencoding;
use crate::bittorrent::NOT_INTERESTED;
use tokio_util::bytes::BufMut;
use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use std::io::Read;

// Bit 20 is set per https://www.bittorrent.org/beps/bep_0010.html
const EXTENSION_RESERVED: u64 = 0b0000_1000_0000_0000_0000_0000;

const EXTENSION_MSG_ID: u8 = 20;

#[derive(Debug, Clone)]
pub enum BittorrentMessage {
    Handshake {
        handshake: BittorrentHandshake,
    },
    ExtensionHandshake {
        metadata_size: Option<u32>,
    },
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
    ExtensionMessage {
        message: ExtensionMessage,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ExtensionMessage {
    Request {
        piece_index: u32,
    },
    Data {
        piece_index: u32,
        total_size: u32,
        data: Vec<u8>,
    },
    Reject {
        piece_index: u32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExtensionRequest {
    pub piece: u32,
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
            EXTENSION => {
                // See https://www.bittorrent.org/beps/bep_0009.html
                let message_id = payload[0];

                let cursor = Cursor::new(&payload[1..payload.len()]);
                let mut data = BufReader::new(cursor);

                let dictionary = match Bencoding::deserialize(&mut data) {
                    Ok(Bencoding::Dictionary { dictionary, .. }) => dictionary,
                    Ok(data) => {
                        return Err(ParseError::UnexpectedType {
                            expected: "dictionary",
                            actual: data.name(),
                        });
                    }
                    Err(_) => return Err(ParseError::Malformed),
                };

                let msg_type = match dictionary.get("msg_type".as_bytes()) {
                    Some(Bencoding::Integer {
                        integer,
                        position: _,
                    }) => *integer as u8,
                    Some(data) => {
                        return Err(ParseError::UnexpectedType {
                            expected: "integer",
                            actual: data.name(),
                        });
                    }
                    None => return Err(ParseError::MissingField { name: "msg_type" }),
                };

                let piece = match dictionary.get("piece".as_bytes()) {
                    Some(Bencoding::Integer {
                        integer,
                        position: _,
                    }) => *integer,
                    Some(data) => {
                        return Err(ParseError::UnexpectedType {
                            expected: "integer",
                            actual: data.name(),
                        });
                    }
                    None => return Err(ParseError::MissingField { name: "piece" }),
                };

                if piece < 0 {
                    todo!("handle this safely");
                }

                if msg_type == EXT_REQUEST {
                    let message = ExtensionMessage::Request {
                        piece_index: piece as u32,
                    };
                    return Ok(BittorrentMessage::ExtensionMessage { message });
                }

                if msg_type == EXT_DATA {
                    // TODO copy the piece data from end of message
                    let total_size = match dictionary.get("total_size".as_bytes()) {
                        Some(Bencoding::Integer {
                            integer,
                            position: _,
                        }) => integer,
                        Some(_) => todo!(),
                        None => return Err(ParseError::MissingField { name: "total_size" }),
                    };

                    let extension_message = ExtensionMessage::Data {
                        piece_index: piece as u32, // TODO might panic
                        total_size: *total_size as u32,
                        data: Vec::new(),
                    };

                    return Ok(BittorrentMessage::ExtensionMessage {
                        message: extension_message,
                    });
                }
                if msg_type == EXT_REJECT {
                    let message = ExtensionMessage::Reject {
                        piece_index: piece as u32,
                    };
                    return Ok(BittorrentMessage::ExtensionMessage { message });
                }

                // TODO This part is really annoying fix it
                // Read the spec

                let message =
                    todo!("Here, we take the bencoded dictionary and take out the entries");

                Ok(BittorrentMessage::ExtensionMessage { message })
            }
            id => Err(ParseError::UnknownId(id)),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        match self {
            BittorrentMessage::Handshake { handshake } => {
                let reserved = handshake.reserved.to_be_bytes();

                let len = 1
                    + handshake.pstr.len()
                    + reserved.len()
                    + handshake.info_hash.len()
                    + handshake.peer_id.len();

                let mut data = Vec::with_capacity(len);
                data.push(len as u8);
                data.extend(&handshake.pstr);
                data.extend(reserved);
                data.extend(handshake.info_hash);
                data.extend(handshake.peer_id);

                data
            }
            BittorrentMessage::ExtensionHandshake { metadata_size } => {
                let mut dictionary: HashMap<Vec<u8>, Bencoding> = HashMap::new();
                if let Some(metadata_size) = metadata_size {
                    dictionary.insert(
                        "metadata_size".as_bytes().to_owned(),
                        Bencoding::Integer {
                            integer: *metadata_size as i64,
                            position: None,
                        },
                    );
                }

                let mut payload = Vec::new();
                payload.push(EXT_HANDSHAKE);
                Bencoding::Dictionary {
                    dictionary,
                    position: None,
                }
                .serialize(&mut payload).expect("unable to serialise dictionary");

                let mut data = Vec::new();
                data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                data.push(EXTENSION);
                data.extend_from_slice(&payload);

                data
            }
            BittorrentMessage::KeepAlive => vec![0, 0, 0, 0],
            BittorrentMessage::Choke => vec![0, 0, 0, 1, 0],
            BittorrentMessage::Unchoke => vec![0, 0, 0, 1, 1],
            BittorrentMessage::Interested => vec![0, 0, 0, 1, 2],
            BittorrentMessage::NotInterested => vec![0, 0, 0, 1, 3],
            BittorrentMessage::Have { piece_index } => {
                let mut v: Vec<u8> = Vec::with_capacity(4 + 1 + 4);
                v.extend_from_slice(&(1u32 + 4u32).to_be_bytes());
                v.push(HAVE);
                v.extend_from_slice(&piece_index.to_be_bytes());
                v
            }
            BittorrentMessage::Bitfield { bitfield } => {
                let mut v = Vec::with_capacity(4 + 1 + bitfield.len());
                v.extend_from_slice(&(1 + bitfield.len() as u32).to_be_bytes());
                v.push(BITFIELD);
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
                v.push(REQUEST);
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
                v.push(PIECE);
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
                v.push(CANCEL);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(&length.to_be_bytes());
                v
            }
            BittorrentMessage::Port { port } => {
                let mut v = Vec::with_capacity(4 + 1 + 2);
                v.extend_from_slice(&(1u32 + 2u32).to_be_bytes());
                v.push(PORT);
                v.extend_from_slice(&port.to_be_bytes());
                v
            }
            BittorrentMessage::ExtensionMessage { message } => {
                let mut message_dict = HashMap::new();
                let mut msg_bytes = Vec::new();

                let (msg_type_int, piece_index) = match message {
                    ExtensionMessage::Request { piece_index, .. } => (EXT_REQUEST, *piece_index),
                    ExtensionMessage::Data { piece_index, .. } => (EXT_DATA, *piece_index),
                    ExtensionMessage::Reject { piece_index, .. } => (EXT_REJECT, *piece_index),
                };

                let msg_type = Bencoding::Integer {
                    integer: msg_type_int as i64,
                    position: None,
                };
                let msg_type_key = "msg_type".as_bytes().to_owned();
                message_dict.insert(msg_type_key, msg_type);

                let piece_index_int = Bencoding::Integer {
                    integer: piece_index as i64,
                    position: None,
                };
                let piece_key = "piece".as_bytes().to_owned();
                message_dict.insert(piece_key, piece_index_int);

                if let ExtensionMessage::Data { data, .. } = message {
                    let total_size = data.len();
                    let total_size_key = "total_size".as_bytes().to_owned();
                    let total_size_integer = Bencoding::Integer {
                        integer: total_size as i64,
                        position: None,
                    };

                    message_dict.insert(total_size_key, total_size_integer);
                }

                let msg = Bencoding::Dictionary {
                    dictionary: message_dict,
                    position: None,
                };
                msg.serialize(&mut msg_bytes).unwrap();

                if let ExtensionMessage::Data { data, .. } = message {
                    msg_bytes.append(&mut data.clone());
                }

                msg_bytes
            }
        }
    }
}

#[derive(Debug)]
pub enum ParseError {
    InvalidLength,
    UnknownId(u8),
    Malformed,
    UnexpectedType {
        expected: &'static str,
        actual: &'static str,
    },
    MissingField {
        name: &'static str,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidLength => write!(f, "invalid length"),
            ParseError::UnknownId(id) => write!(f, "unknown id {}", id),
            ParseError::Malformed => write!(f, "malformed"),
            ParseError::UnexpectedType { expected, actual } => {
                write!(f, "expected {}, received {}", expected, actual)
            }
            ParseError::MissingField { name } => write!(f, "missing field {}", name),
        }
    }
}

impl error::Error for ParseError {}

#[derive(Debug, Clone)]
pub struct BittorrentHandshake {
    pub pstr: Vec<u8>,
    pub reserved: u64,
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl BittorrentHandshake {
    pub fn new(info_hash: [u8; 20], peer_id: [u8; 20]) -> Self {
        Self {
            reserved: EXTENSION_RESERVED,
            info_hash,
            peer_id,
            pstr: "BitTorrent Protocol".as_bytes().to_owned(),
        }
    }

    pub fn extension(&self) -> bool {
        self.reserved & EXTENSION_RESERVED > 0
    }
}

const RESERVED_LEN: usize = 8;
const INFO_HASH_LEN: usize = 20;
const PEER_ID_LEN: usize = 20;

impl BittorrentHandshake {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(49 + 20 + 20);
        buf.push(19);
        buf.extend_from_slice(b"BitTorrent protocol");
        buf.put_u64(EXTENSION_RESERVED);
        buf.extend_from_slice(&[0; 8]);
        buf.extend_from_slice(&self.info_hash);
        buf.extend_from_slice(&self.peer_id);
        buf
    }

    pub fn deserialise(data: &[u8]) -> Result<Option<BittorrentHandshake>, ParseError> {
        const RESERVED_LEN: usize = 8;
        const INFO_HASH_LEN: usize = 20;
        const PEER_ID_LEN: usize = 20;

        if data.len() == 0 {
            return Ok(None);
        }

        let pstr_len = data[0] as usize;
        let total_msg_len = 1 + pstr_len + RESERVED_LEN + INFO_HASH_LEN + PEER_ID_LEN;
        if data.len() < total_msg_len {
            return Ok(None);
        }

        let mut cursor = Cursor::new(&data[1..data.len()]);
        let pstr_len = cursor.read_u8().unwrap() as usize;
        let mut pstr_bytes = vec![0u8; pstr_len];
        cursor.read_exact(&mut pstr_bytes).unwrap();

        let reserved = cursor.read_u64::<BigEndian>().unwrap();

        let mut info_hash = [0u8; 20];
        cursor.read_exact(&mut info_hash).unwrap();

        let mut peer_id = [0u8; 20];
        cursor.read_exact(&mut peer_id).unwrap();

        Ok(Some(BittorrentHandshake {
            pstr: pstr_bytes,
            reserved,
            info_hash,
            peer_id,
        }))
    }
}

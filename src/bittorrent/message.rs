use std::collections::HashMap;
use std::error;
use std::fmt;
use std::io::BufReader;
use std::io::Cursor;
use std::string::FromUtf8Error;

use super::ids::*;
use crate::bittorrent::Bencoding;
use crate::bittorrent::NOT_INTERESTED;
use crate::bittorrent::PeerId;
use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use log::trace;
use std::io::Read;

// Bit 20 is set per https://www.bittorrent.org/beps/bep_0010.html
const EXTENSION_RESERVED: u64 = 0b0000_1000_0000_0000_0000_0000;

const EXTENSION_MSG_ID: u8 = 20;

#[derive(Debug, Clone)]
pub enum BittorrentMessage {
    Handshake {
        handshake: BittorrentHandshake,
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
    Extension {
        message: ExtensionMessage,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ExtensionMessage {
    Handshake {
        metadata_size: Option<u32>,
        ut_metadata: u32,
    },
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

#[derive(Clone)]
pub struct MessageDeserialiser {
    handshake: bool,
    ut_metadata: Option<u32>,
}

impl MessageDeserialiser {
    pub fn new() -> Self {
        Self {
            ut_metadata: None,
            handshake: false,
        }
    }

    pub fn deserialise(&mut self, data: &[u8]) -> Result<BittorrentMessage, ParseError> {
        if data.len() == 0 {
            trace!("Length is zero, that's way too short");
            return Err(ParseError::InvalidLength);
        }

        if !self.handshake {
            return match BittorrentHandshake::deserialise(data)? {
                Some(handshake) => {
                    self.handshake = true;

                    Ok(BittorrentMessage::Handshake { handshake })
                }
                None => Err(ParseError::InvalidLength),
            };
        }

        if data.len() < 4 {
            trace!("Length is less than 4, that's way too short");
            return Err(ParseError::InvalidLength);
        }
        let length = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;

        if length == 0 {
            return Ok(BittorrentMessage::KeepAlive);
        }

        if data.len() < 4 + length {
            trace!(
                "Length was {} but must be {} at minimum",
                data.len(),
                4 + length
            );
            return Err(ParseError::InvalidLength);
        }

        let id = data[4];
        let payload = &data[5..4 + length];

        match id {
            CHOKE => return Ok(BittorrentMessage::Choke),
            UNCHOKE => return Ok(BittorrentMessage::Unchoke),
            INTERESTED => return Ok(BittorrentMessage::Interested),
            NOT_INTERESTED => return Ok(BittorrentMessage::NotInterested),
            HAVE => {
                if payload.len() != 4 {
                    return Err(ParseError::InvalidLength);
                }
                let piece_index =
                    u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                return Ok(BittorrentMessage::Have { piece_index });
            }
            BITFIELD => {
                return Ok(BittorrentMessage::Bitfield {
                    bitfield: payload.to_vec(),
                });
            }
            REQUEST => {
                if payload.len() != 12 {
                    return Err(ParseError::InvalidLength);
                }
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let length = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);

                return Ok(BittorrentMessage::Request {
                    index,
                    begin,
                    length,
                });
            }
            PIECE => {
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let block = payload[8..payload.len()].to_vec();

                return Ok(BittorrentMessage::Piece {
                    index,
                    begin,
                    block,
                });
            }
            CANCEL => {
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let length = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);

                return Ok(BittorrentMessage::Cancel {
                    index,
                    begin,
                    length,
                });
            }
            PORT => {
                let port = u16::from_be_bytes([payload[0], payload[1]]);

                return Ok(BittorrentMessage::Port { port });
            }
            EXTENSION => {
                // See https://www.bittorrent.org/beps/bep_0009.html
                let message_id = payload[0];

                let cursor = Cursor::new(&payload[1..payload.len()]);
                let mut data = BufReader::new(cursor);

                let message = match (message_id, self.ut_metadata) {
                    (EXT_HANDSHAKE, _) => {
                        let dictionary = match Bencoding::deserialise(&mut data) {
                            Ok(Bencoding::Dictionary { dictionary, .. }) => dictionary,
                            Ok(data) => {
                                return Err(ParseError::UnexpectedType {
                                    expected: "dictionary",
                                    actual: data.name(),
                                });
                            }
                            Err(_) => return Err(ParseError::Malformed),
                        };

                        let m = match dictionary.get("m".as_bytes()) {
                            Some(Bencoding::Dictionary {
                                dictionary: m,
                                position: _,
                            }) => m,
                            None => return Err(ParseError::MissingField { name: "m" }),
                            Some(d) => {
                                return Err(ParseError::UnexpectedType {
                                    expected: "dictionary",
                                    actual: d.name(),
                                });
                            }
                        };

                        let ut_metadata = match m.get("ut_metadata".as_bytes()) {
                            Some(Bencoding::Integer {
                                integer,
                                position: _,
                            }) => *integer as u32,
                            Some(data) => {
                                return Err(ParseError::UnexpectedType {
                                    expected: "integer",
                                    actual: data.name(),
                                });
                            }
                            None => {
                                return Err(ParseError::MissingField {
                                    name: "ut_metadata",
                                });
                            }
                        };

                        let metadata_size = match dictionary.get("metadata_size".as_bytes()) {
                            Some(Bencoding::Integer {
                                integer,
                                position: _,
                            }) => Some(*integer as u32),
                            Some(data) => {
                                return Err(ParseError::UnexpectedType {
                                    expected: "integer",
                                    actual: data.name(),
                                });
                            }
                            None => None,
                        };

                        ExtensionMessage::Handshake {
                            metadata_size,
                            ut_metadata,
                        }
                    }
                    (message_id, Some(ut_metadata)) => {
                        if message_id as u32 != ut_metadata {
                            return Err(ParseError::UnknownExtId(message_id));
                        }

                        let cursor = Cursor::new(payload);
                        let mut data = BufReader::new(cursor); // TODO just reuse the reader

                        let dictionary = match Bencoding::deserialise(&mut data) {
                            Ok(Bencoding::Dictionary { dictionary, .. }) => dictionary,
                            Ok(other) => {
                                return Err(ParseError::UnexpectedType {
                                    expected: "dictionary",
                                    actual: other.name(),
                                });
                            }
                            Err(_) => return Err(ParseError::Malformed),
                        };

                        let metadata_message_type = match dictionary.get("msg_type".as_bytes()) {
                            Some(Bencoding::Integer { integer, .. }) => integer,
                            Some(other) => {
                                return Err(ParseError::UnexpectedType {
                                    expected: "dictionary",
                                    actual: other.name(),
                                });
                            }
                            None => return Err(ParseError::MissingField { name: "msg_type" }),
                        };

                        let piece_index = match dictionary.get("piece".as_bytes()) {
                            Some(Bencoding::Integer { integer, .. }) => integer,
                            Some(other) => {
                                return Err(ParseError::UnexpectedType {
                                    expected: "integer",
                                    actual: other.name(),
                                });
                            }
                            None => return Err(ParseError::MissingField { name: "piece" }),
                        };

                        let message = match *metadata_message_type as u8 {
                            EXT_REQUEST => ExtensionMessage::Request {
                                piece_index: *piece_index as u32,
                            }, // TODO
                            EXT_DATA => {
                                let mut piece_data = Vec::with_capacity(1024 * 4);
                                data.read_to_end(&mut piece_data)
                                    .expect("unable to read piece data"); // TODO

                                let total_size = match dictionary.get("total_size".as_bytes()) {
                                    Some(Bencoding::Integer { integer, .. }) => integer,
                                    Some(other) => {
                                        return Err(ParseError::UnexpectedType {
                                            expected: "integer",
                                            actual: other.name(),
                                        });
                                    }
                                    None => {
                                        return Err(ParseError::MissingField {
                                            name: "total_size",
                                        });
                                    }
                                };

                                ExtensionMessage::Data {
                                    piece_index: *piece_index as u32,
                                    total_size: *total_size as u32,
                                    data: piece_data,
                                }
                            }
                            EXT_REJECT => ExtensionMessage::Reject {
                                piece_index: *piece_index as u32,
                            },
                            _ => todo!(),
                        };

                        return Ok(BittorrentMessage::Extension { message });
                    }
                    (unknown_ext_id, _) => return Err(ParseError::UnknownExtId(unknown_ext_id)),
                };

                return Ok(BittorrentMessage::Extension { message });
            }
            unknown_id => return Err(ParseError::UnknownId(unknown_id)),
        }
    }
}

#[derive(Clone)]
pub struct MessageSerialiser;

impl MessageSerialiser {
    pub fn new() -> Self {
        Self {}
    }

    pub fn serialise(&self, message: &BittorrentMessage) -> Vec<u8> {
        match message {
            BittorrentMessage::Handshake { handshake } => {
                let reserved = handshake.reserved.to_be_bytes();

                let len = 1
                    + handshake.pstr.len()
                    + reserved.len()
                    + handshake.info_hash.len()
                    + handshake.peer_id.len();

                let mut data = Vec::with_capacity(len);
                data.push(handshake.pstr.len() as u8); // TODO
                data.extend(&handshake.pstr);
                data.extend(reserved);
                data.extend(handshake.info_hash);
                data.extend(handshake.peer_id);

                trace!("Serialised handshake of length {}", data.len());

                data
            }
            /*BittorrentMessage::ExtensionHandshake { metadata_size } => {
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
            }*/
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
            BittorrentMessage::Extension {
                message: ExtensionMessage::Handshake { metadata_size, .. },
            } => {
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
                .serialise(&mut payload)
                .expect("unable to serialise dictionary");

                let mut data = Vec::new();
                data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                data.push(EXTENSION);
                data.extend_from_slice(&payload);

                data
            }
            BittorrentMessage::Extension { message } => {
                let mut message_dict = HashMap::new();
                let mut msg_bytes = Vec::new();

                let (msg_type_int, piece_index) = match message {
                    ExtensionMessage::Request { piece_index, .. } => (EXT_REQUEST, *piece_index),
                    ExtensionMessage::Data { piece_index, .. } => (EXT_DATA, *piece_index),
                    ExtensionMessage::Reject { piece_index, .. } => (EXT_REJECT, *piece_index),
                    ExtensionMessage::Handshake { .. } => unreachable!(),
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
                msg.serialise(&mut msg_bytes).unwrap();

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
    UnknownExtId(u8),
    Malformed,
    UnexpectedType {
        expected: &'static str,
        actual: &'static str,
    },
    MissingField {
        name: &'static str,
    },
    TooLong,
}

impl From<FromUtf8Error> for ParseError {
    fn from(_err: FromUtf8Error) -> Self {
        Self::InvalidLength
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidLength => write!(f, "invalid length"),
            ParseError::UnknownId(id) => write!(f, "unknown id {}", id),
            ParseError::UnknownExtId(id) => write!(f, "unknown extension msg id {}", id),
            ParseError::Malformed => write!(f, "malformed"),
            ParseError::UnexpectedType { expected, actual } => {
                write!(f, "expected {}, received {}", expected, actual)
            }
            ParseError::MissingField { name } => write!(f, "missing field {}", name),
            ParseError::TooLong => write!(f, "message length exceeded bounds"),
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

const MAX_PSTR: usize = 1024;

impl BittorrentHandshake {
    pub fn new(peer_id: PeerId, info_hash: [u8; 20]) -> Self {
        // the extension protocol is enabled

        BittorrentHandshake {
            pstr: "BitTorrent protocol".as_bytes().to_owned(),
            reserved: EXTENSION_RESERVED,
            info_hash,
            peer_id: peer_id.0,
        }
    }

    pub fn extension(&self) -> bool {
        self.reserved & EXTENSION_RESERVED == EXTENSION_RESERVED
    }

    pub fn deserialise(data: &[u8]) -> Result<Option<Self>, ParseError> {
        trace!("Deserialising handshake from data of length {}", data.len());
        if data.len() == 0 {
            trace!("Data length is zero");
            return Ok(None);
        }

        let mut cursor = Cursor::new(&data);

        let pstr_len = cursor.read_u8().expect("unable to read first byte") as usize;
        let handshake_len = pstr_len + 49;
        if data.len() < handshake_len {
            trace!(
                "Data was {} byte(s) but need {} bytes",
                data.len() - 1,
                handshake_len
            );
            return Ok(None);
        }

        if pstr_len > MAX_PSTR {
            return Err(ParseError::TooLong);
        }

        let mut pstr = vec![0u8; pstr_len];
        cursor.read_exact(&mut pstr).expect("unable to read pstr");

        let mut info_hash = [0u8; 20];
        let mut peer_id = [0u8; 20];

        let reserved = cursor
            .read_u64::<BigEndian>()
            .expect("unable to read reserved");
        cursor
            .read_exact(&mut info_hash)
            .expect("unable to read info_hash");
        cursor
            .read_exact(&mut peer_id)
            .expect("unable to read peer_id");

        let handshake = Self {
            pstr,
            reserved,
            info_hash,
            peer_id,
        };

        trace!("Deserialised handshake: {:?}", handshake);

        Ok(Some(handshake))
    }
}

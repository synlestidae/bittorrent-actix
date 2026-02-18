use core::fmt;
use std::error;

use log::trace;
use tokio::io;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::bittorrent::message::{MessageDeserialiser, MessageSerialiser};

use super::{BittorrentMessage, ExtensionMessage, ParseError};

#[derive(Clone)]
pub struct BittorrentCodec {
    handshake_received: bool,
    extension_data_message: Option<ExtensionDataMessageState>,
    serialiser: MessageSerialiser,
    deserialiser: MessageDeserialiser,
}

#[derive(Clone)]
struct ExtensionDataMessageState {
    piece_index: u32,
    total_size: u32,
}

impl BittorrentCodec {
    pub fn new() -> Self {
        Self {
            handshake_received: false,
            extension_data_message: None,
            serialiser: MessageSerialiser::new(),
            deserialiser: MessageDeserialiser::new(),
        }
    }
}

impl Decoder for BittorrentCodec {
    type Item = Vec<BittorrentMessage>;
    type Error = DecoderError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        trace!("Decoding from {} byte(s)", src.len());

        if src.len() == 0 {
            trace!("No bytes to decode");
            return Ok(None);
        }

        if let Some(ExtensionDataMessageState {
            piece_index,
            total_size,
        }) = self.extension_data_message.clone()
        {
            let message_end_ext = total_size as usize;
            if message_end_ext < src.len() {
                return Ok(None); // wait for more data
            }

            let data = src.split_to(total_size as usize).to_vec();

            return Ok(Some(vec![BittorrentMessage::Extension {
                message: ExtensionMessage::Data {
                    piece_index,
                    total_size,
                    data,
                },
            }]));
        }

        let mut messages = Vec::new();

        if !self.handshake_received {
            trace!("Need to decode handshake first");

            let pstr_len = *(src.get(0).unwrap()) as usize;
            let handshake_len = pstr_len + 49;

            if src.len() < handshake_len {
                trace!(
                    "Src was {} byte(s) but need {} bytes",
                    src.len() - 1,
                    handshake_len
                );
                return Ok(None);
            }
            trace!("Deserialising {} handshake byte(s)", handshake_len);

            let data = src.split_to(handshake_len);
            if let BittorrentMessage::Handshake { handshake } =
                self.deserialiser.deserialise(&data.to_vec())?
            {
                self.handshake_received = true; // TODO redundant, duplicated 
                messages.push(BittorrentMessage::Handshake { handshake });
            } else {
                todo!("handshake should be first")
            }

            return Ok(Some(messages));
        }

        // message len is u32 bit (8 byte) integer
        while src.len() > 4 {
            trace!("Have {} byte(s)", src.len());
            let message_len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);

            let message_end = message_len as usize + 4;
            if src.len() < message_end {
                if messages.len() == 0 {
                    return Ok(None);
                }
                return Ok(Some(messages));
            }

            // read the message, but data will be empty
            let msg = self.deserialiser.deserialise(&src.split_to(message_end))?;
            match msg {
                BittorrentMessage::Extension {
                    message:
                        ExtensionMessage::Data {
                            piece_index,
                            total_size,
                            data: _,
                        },
                } => {
                    // if we do not have enough data, save state and come back
                    let message_end_ext = total_size as usize;
                    if message_end_ext < src.len() {
                        self.extension_data_message = Some(ExtensionDataMessageState {
                            piece_index,
                            total_size,
                        });

                        return Ok(None);
                    }

                    let data = src.split_to(message_end_ext).to_vec();

                    let message = ExtensionMessage::Data {
                        piece_index,
                        total_size,
                        data,
                    };

                    return Ok(Some(vec![BittorrentMessage::Extension { message }]));
                }
                _ => (),
            }
            messages.push(msg);
        }

        if messages.is_empty() {
            return Ok(None);
        }

        Ok(Some(messages))
    }
}

impl Encoder<BittorrentMessage> for BittorrentCodec {
    type Error = io::Error;

    fn encode(&mut self, item: BittorrentMessage, dst: &mut BytesMut) -> Result<(), Self::Error> {
        trace!("Sending {:?}", item);

        dst.extend_from_slice(&self.serialiser.serialise(&item));

        Ok(())
    }
}

#[derive(Debug)]
pub enum DecoderError {
    Io(io::Error),
    Parse(ParseError),
}

impl fmt::Display for DecoderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecoderError::Io(err) => err.fmt(f),
            DecoderError::Parse(err) => err.fmt(f),
        }
    }
}

impl error::Error for DecoderError {}

impl From<io::Error> for DecoderError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<ParseError> for DecoderError {
    fn from(err: ParseError) -> Self {
        Self::Parse(err)
    }
}

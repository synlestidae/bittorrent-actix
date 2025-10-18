use core::fmt;
use std::error;

use log::trace;
use tokio::io;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use super::{BittorrentHandshake, BittorrentMessage, ParseError};

#[derive(Clone)]
pub struct BittorrentCodec {
    handshake_sent: bool,
    handshake_received: bool,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl BittorrentCodec {
    pub fn new(info_hash: [u8; 20], peer_id: [u8; 20]) -> Self {
        Self {
            handshake_sent: false,
            handshake_received: false,
            info_hash,
            peer_id,
        }
    }
}

impl Decoder for BittorrentCodec {
    type Item = Vec<BittorrentMessage>;
    type Error = DecoderError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !self.handshake_received {
            trace!("Decoding handshake first");
            if src.is_empty() {
                trace!("Have 0 bytes for handshake but need more");
                return Ok(None);
            }

            const RESERVED_LEN: usize = 8;
            const INFO_HASH_LEN: usize = 20;
            const PEER_ID_LEN: usize = 20;

            let pstr_len = src[0] as usize;
            let total_msg_len = 1 + pstr_len + RESERVED_LEN + INFO_HASH_LEN + PEER_ID_LEN;
            if src.len() < total_msg_len {
                trace!(
                    "Have {} byte(s) for handshake but need {}",
                    src.len(),
                    total_msg_len
                );
                return Ok(None);
            }

            // don't actually care, just read off the bytes and drop them
            drop(src.split_to(total_msg_len));

            trace!("Successfully decoded handshake");
            self.handshake_received = true;

            return Ok(None);
        }

        // check we have enough bytes
        let mut messages = Vec::new();
        while src.len() > 4 {
            trace!("Have {} byte(s)", src.len());
            let message_len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);
            let total_len = message_len as usize + 4;
            if src.len() < total_len {
                if messages.len() == 0 {
                    return Ok(None);
                }
                return Ok(Some(messages));
            }

            // read the message
            let msg = BittorrentMessage::deserialize(&src.split_to(total_len))?;
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
        if !self.handshake_sent {
            trace!("Sending handshake first");

            let handshake = BittorrentHandshake {
                info_hash: self.info_hash,
                peer_id: self.peer_id,
            };
            dst.extend_from_slice(&handshake.serialize());

            self.handshake_sent = true;
        }

        trace!("Sending {:?}", item);

        dst.extend_from_slice(&item.serialize());

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

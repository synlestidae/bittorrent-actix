use std::io::Read;
use std::io::Write;
use std::{
    io::{self, Cursor},
    net::Ipv4Addr,
    time::Duration,
};
pub use tokio::net::UdpSocket;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

pub struct UdpClient {
    host: String,
    port: u16,
    socket: UdpSocket,
    timeout: Duration,
}

#[derive(Debug)]
pub enum UdpClientErr {
    Io(io::Error),
    ParseErr(ParseErr),
}

impl From<io::Error> for UdpClientErr {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<ParseErr> for UdpClientErr {
    fn from(err: ParseErr) -> Self {
        Self::ParseErr(err)
    }
}

impl UdpClient {
    pub async fn connect(host: &str, port: u16) -> Result<Self, UdpClientErr> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect((host, port));

        let timeout = Duration::from_secs(30);

        Ok(Self {
            host: host.to_owned(),
            port,
            socket,
            timeout,
        })
    }

    pub async fn send<M: UdpMessage>(&self, message: M) -> Result<(), UdpClientErr> {
        let data: Vec<u8> = message.serialize();

        self.socket.send(&data).await?;

        Ok(())
    }

    pub async fn recv<M: UdpMessage>(&self) -> Result<M, UdpClientErr> {
        let mut buffer = vec![0u8; M::buffer_len()];

        self.socket.recv(&mut buffer).await?;

        let message: M = M::deserialise(&buffer)?;
        Ok(message)
    }
}

#[derive(Debug)]
pub enum ParseErr {
    Io(io::Error),
    TooShort,
}

impl From<io::Error> for ParseErr {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

trait UdpMessage: Sized {
    fn serialize(&self) -> Vec<u8>;
    fn deserialise(data: &[u8]) -> Result<Self, ParseErr>;
    fn buffer_len() -> usize;
}

pub const UDP_PROTOCOL_ID: u64 = 0x41727101980;

pub const CONNECT_ACTION: u32 = 0;

pub const ACCOUNT_ACTION: u32 = 1;

/// Offset  Size            Name            Value
/// 0       64-bit integer  protocol_id     0x41727101980 // magic constant
/// 8       32-bit integer  action          0 // connect
/// 12      32-bit integer  transaction_id
/// 16
pub struct ConnectReq {
    pub protocol_id: u64,
    pub action: u32,
    pub transaction_id: u32,
}

impl UdpMessage for ConnectReq {
    fn deserialise(data: &[u8]) -> Result<Self, ParseErr> {
        let mut cursor = Cursor::new(data);
        let protocol_id = cursor.read_u64::<BigEndian>()?;
        let action = cursor.read_u32::<BigEndian>()?;
        let transaction_id = cursor.read_u32::<BigEndian>()?;

        Ok(Self {
            protocol_id,
            action,
            transaction_id,
        })
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        data.write_u64::<BigEndian>(self.protocol_id);
        data.write_u32::<BigEndian>(self.action);
        data.write_u32::<BigEndian>(self.transaction_id);

        data
    }

    fn buffer_len() -> usize {
        16
    }
}

/// Offset  Size            Name            Value
/// 0       32-bit integer  action          0 // connect
/// 4       32-bit integer  transaction_id
/// 8       64-bit integer  connection_id
/// 16
pub struct ConnectRes {
    pub action: u32,
    pub transaction_id: u32,
    pub connection_id: u64,
}

impl UdpMessage for ConnectRes {
    fn deserialise(data: &[u8]) -> Result<Self, ParseErr> {
        let mut cursor = Cursor::new(data);
        let action = cursor.read_u32::<BigEndian>()?;
        let transaction_id = cursor.read_u32::<BigEndian>()?;
        let connection_id = cursor.read_u64::<BigEndian>()?;

        Ok(Self {
            action,
            transaction_id,
            connection_id,
        })
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        data.write_u32::<BigEndian>(self.action);
        data.write_u32::<BigEndian>(self.transaction_id);
        data.write_u64::<BigEndian>(self.connection_id);

        data
    }

    fn buffer_len() -> usize {
        16
    }
}

pub const ANNOUNCE_ACTION: u32 = 0;

pub const NONE_EVENT: u32 = 0;
pub const COMPLETED_EVENT: u32 = 1;
pub const STARTEED_EVENT: u32 = 2;
pub const STOPPED_EVENT: u32 = 3;

/// Offset  Size    Name    Value
/// 0       64-bit integer  connection_id
/// 8       32-bit integer  action          1 // announce
/// 12      32-bit integer  transaction_id
/// 16      20-byte string  info_hash
/// 36      20-byte string  peer_id
/// 56      64-bit integer  downloaded
/// 64      64-bit integer  left
/// 72      64-bit integer  uploaded
/// 80      32-bit integer  event           0 // 0: none; 1: completed; 2: started; 3: stopped
/// 84      32-bit integer  IP address      0 // default
/// 88      32-bit integer  key
/// 92      32-bit integer  num_want        -1 // default
/// 96      16-bit integer  port
/// 98
pub struct AnnounceReq {
    pub connection_id: u64,
    pub action: u32,
    pub transaction_id: u32,
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
    pub downloaded: u64,
    pub left: u64,
    pub uploaded: u64,
    pub event: u32,
    pub ip_address: u32,
    pub key: u32,
    pub num_want: i32,
    pub port: u16,
}

impl UdpMessage for AnnounceReq {
    fn deserialise(data: &[u8]) -> Result<Self, ParseErr> {
        let mut cursor = Cursor::new(data);
        let connection_id = cursor.read_u64::<BigEndian>()?;
        let action = cursor.read_u32::<BigEndian>()?;
        let transaction_id = cursor.read_u32::<BigEndian>()?;

        let mut info_hash: [u8; 20] = [0u8; 20];
        cursor.read_exact(&mut info_hash)?;
        let mut peer_id: [u8; 20] = [0u8; 20];
        cursor.read_exact(&mut peer_id)?;

        let downloaded = cursor.read_u64::<BigEndian>()?;
        let left = cursor.read_u64::<BigEndian>()?;
        let uploaded = cursor.read_u64::<BigEndian>()?;
        let event = cursor.read_u32::<BigEndian>()?;
        let ip_address = cursor.read_u32::<BigEndian>()?;
        let key = cursor.read_u32::<BigEndian>()?;
        let num_want = cursor.read_i32::<BigEndian>()?;
        let port = cursor.read_u16::<BigEndian>()?;

        Ok(Self {
            connection_id,
            action,
            transaction_id,
            info_hash,
            peer_id,
            downloaded,
            left,
            uploaded,
            event,
            ip_address,
            key,
            num_want,
            port,
        })
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.write_u64::<BigEndian>(self.connection_id);
        data.write_u32::<BigEndian>(self.action);
        data.write_u32::<BigEndian>(self.transaction_id);

        data.write_all(&self.info_hash);
        data.write_all(&self.peer_id);

        data.write_u64::<BigEndian>(self.downloaded);
        data.write_u64::<BigEndian>(self.left);
        data.write_u64::<BigEndian>(self.uploaded);
        data.write_u32::<BigEndian>(self.event);
        data.write_u32::<BigEndian>(self.ip_address);
        data.write_u32::<BigEndian>(self.key);
        data.write_i32::<BigEndian>(self.num_want);
        data.write_u16::<BigEndian>(self.port);

        data
    }

    fn buffer_len() -> usize {
        98
    }
}

/// Offset      Size            Name            Value
/// 0           32-bit integer  action          1 // announce
/// 4           32-bit integer  transaction_id
/// 8           32-bit integer  interval
/// 12          32-bit integer  leechers
/// 16          32-bit integer  seeders
/// 20 + 6 * n  32-bit integer  IP address
/// 24 + 6 * n  16-bit integer  TCP port
/// 20 + 6 * N
pub struct AnnounceRes {
    pub action: u32,
    pub transaction_id: u32,
    pub interval: u32,
    pub leechers: u32,
    pub seeders: u32,
    pub peers: Vec<(Ipv4Addr, u16)>,
}

impl UdpMessage for AnnounceRes {
    fn deserialise(data: &[u8]) -> Result<Self, ParseErr> {
        let len = data.len();

        let mut cursor = Cursor::new(data);

        let action = cursor.read_u32::<BigEndian>()?;
        let transaction_id = cursor.read_u32::<BigEndian>()?;
        let interval = cursor.read_u32::<BigEndian>()?;
        let leechers = cursor.read_u32::<BigEndian>()?;
        let seeders = cursor.read_u32::<BigEndian>()?;

        let mut announce_res = AnnounceRes {
            action,
            transaction_id,
            interval,
            leechers,
            seeders,
            peers: Vec::new(),
        };

        while cursor.position() >= len as u64 {
            let mut ip_address = [0u8; 4];
            match cursor.read(&mut ip_address)? {
                bytes_read => {
                    if bytes_read < 4 {
                        return Err(ParseErr::TooShort);
                    }
                }
            }

            let mut port_bytes = [0u8; 2];
            match cursor.read(&mut port_bytes)? {
                bytes_read => {
                    if bytes_read < 2 {
                        return Err(ParseErr::TooShort);
                    }
                }
            }
            let ip_address =
                Ipv4Addr::new(ip_address[0], ip_address[1], ip_address[2], ip_address[3]);
            let port = ((port_bytes[0] as u16) << 8) + port_bytes[1] as u16;

            announce_res.peers.push((ip_address, port));
        }

        Ok(announce_res)
    }

    fn serialize(&self) -> Vec<u8> {
        unimplemented!()
    }

    fn buffer_len() -> usize {
        const MAX_ADDRESSES: usize = 2048;

        20 + 6 * MAX_ADDRESSES
    }
}

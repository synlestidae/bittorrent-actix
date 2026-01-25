pub struct ConnectionState {
    pub handshake_sent: bool,
    pub handshake_received: bool,
    pub bitfield_sent: bool,
    pub them_choking: bool,
    pub them_interested: bool,
    pub us_choking: bool,
    pub us_interested: bool,
}

impl ConnectionState {
    pub fn new() -> Self {
        Self {
            handshake_sent: false,
            handshake_received: false,
            bitfield_sent: false,
            them_choking: false,
            them_interested: false,
            us_choking: false,
            us_interested: false,
        }
    }
}

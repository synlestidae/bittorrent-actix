#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub index: u32,
    pub begin: u32,
    pub block: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PieceRequest {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletePiece {
    pub index: u32,
    pub data: Vec<u8>,
}

use bytes::{Buf, BufMut, Bytes, BytesMut};
use crate::error::{ProtocolError, Result};

pub const MSG_CHOKE: u8          = 0;
pub const MSG_UNCHOKE: u8        = 1;
pub const MSG_INTERESTED: u8     = 2;
pub const MSG_NOT_INTERESTED: u8 = 3;
pub const MSG_HAVE: u8           = 4;
pub const MSG_BITFIELD: u8       = 5;
pub const MSG_REQUEST: u8        = 6;
pub const MSG_PIECE: u8          = 7;
pub const MSG_CANCEL: u8         = 8;
pub const MSG_HASH_REQUEST: u8   = 21;
pub const MSG_HASHES: u8         = 22;
pub const MSG_HASH_REJECT: u8    = 23;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub pstr: Vec<u8>,
    pub reserved: [u8; 8],
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl Handshake {
    pub const PSTR: &'static [u8] = b"BitTorrent protocol";
    pub const LEN: usize = 68;

    pub fn new(info_hash: [u8; 20], peer_id: [u8; 20], reserved: [u8; 8]) -> Self {
        Self { pstr: Self::PSTR.to_vec(), reserved, info_hash, peer_id }
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(Self::LEN);
        buf.put_u8(self.pstr.len() as u8);
        buf.put_slice(&self.pstr);
        buf.put_slice(&self.reserved);
        buf.put_slice(&self.info_hash);
        buf.put_slice(&self.peer_id);
        buf.freeze()
    }

    pub fn decode(mut buf: impl Buf) -> Result<Self> {
        if buf.remaining() < Self::LEN {
            return Err(ProtocolError::InvalidMessageLength { expected: Self::LEN, got: buf.remaining() });
        }
        let pstr_len = buf.get_u8() as usize;
        let mut pstr = vec![0u8; pstr_len];
        buf.copy_to_slice(&mut pstr);
        let mut reserved = [0u8; 8]; buf.copy_to_slice(&mut reserved);
        let mut info_hash = [0u8; 20]; buf.copy_to_slice(&mut info_hash);
        let mut peer_id = [0u8; 20]; buf.copy_to_slice(&mut peer_id);
        Ok(Self { pstr, reserved, info_hash, peer_id })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockInfo {
    pub piece_index: u32,
    pub begin: u32,
    pub length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    KeepAlive,
    Choke, Unchoke, Interested, NotInterested,
    Have { piece_index: u32 },
    Bitfield(Bytes),
    Request(BlockInfo),
    Piece { piece_index: u32, begin: u32, data: Bytes },
    Cancel(BlockInfo),
    HashRequest { pieces_root: [u8; 32], base_layer: u32, index: u32, length: u32, proof_layers: u32 },
    Hashes     { pieces_root: [u8; 32], base_layer: u32, index: u32, length: u32, proof_layers: u32, hashes: Bytes },
    HashReject { pieces_root: [u8; 32], base_layer: u32, index: u32, length: u32, proof_layers: u32 },
}

impl Message {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();
        match self {
            Message::KeepAlive       => {}
            Message::Choke           => buf.put_u8(MSG_CHOKE),
            Message::Unchoke         => buf.put_u8(MSG_UNCHOKE),
            Message::Interested      => buf.put_u8(MSG_INTERESTED),
            Message::NotInterested   => buf.put_u8(MSG_NOT_INTERESTED),
            Message::Have { piece_index } => { buf.put_u8(MSG_HAVE); buf.put_u32(*piece_index); }
            Message::Bitfield(bits)  => { buf.put_u8(MSG_BITFIELD); buf.put_slice(bits); }
            Message::Request(b)      => { buf.put_u8(MSG_REQUEST); buf.put_u32(b.piece_index); buf.put_u32(b.begin); buf.put_u32(b.length); }
            Message::Piece { piece_index, begin, data } => { buf.put_u8(MSG_PIECE); buf.put_u32(*piece_index); buf.put_u32(*begin); buf.put_slice(data); }
            Message::Cancel(b)       => { buf.put_u8(MSG_CANCEL); buf.put_u32(b.piece_index); buf.put_u32(b.begin); buf.put_u32(b.length); }
            Message::HashRequest { pieces_root, base_layer, index, length, proof_layers } => {
                buf.put_u8(MSG_HASH_REQUEST); buf.put_slice(pieces_root);
                buf.put_u32(*base_layer); buf.put_u32(*index); buf.put_u32(*length); buf.put_u32(*proof_layers);
            }
            Message::Hashes { pieces_root, base_layer, index, length, proof_layers, hashes } => {
                buf.put_u8(MSG_HASHES); buf.put_slice(pieces_root);
                buf.put_u32(*base_layer); buf.put_u32(*index); buf.put_u32(*length); buf.put_u32(*proof_layers);
                buf.put_slice(hashes);
            }
            Message::HashReject { pieces_root, base_layer, index, length, proof_layers } => {
                buf.put_u8(MSG_HASH_REJECT); buf.put_slice(pieces_root);
                buf.put_u32(*base_layer); buf.put_u32(*index); buf.put_u32(*length); buf.put_u32(*proof_layers);
            }
        }
        buf.freeze()
    }

    pub fn decode(mut buf: Bytes) -> Result<Self> {
        if buf.is_empty() { return Ok(Message::KeepAlive); }
        let id = buf.get_u8();
        match id {
            MSG_CHOKE          => Ok(Message::Choke),
            MSG_UNCHOKE        => Ok(Message::Unchoke),
            MSG_INTERESTED     => Ok(Message::Interested),
            MSG_NOT_INTERESTED => Ok(Message::NotInterested),
            MSG_HAVE    => { chk(&buf, 4)?; Ok(Message::Have { piece_index: buf.get_u32() }) }
            MSG_BITFIELD => Ok(Message::Bitfield(buf)),
            MSG_REQUEST => { chk(&buf, 12)?; Ok(Message::Request(BlockInfo { piece_index: buf.get_u32(), begin: buf.get_u32(), length: buf.get_u32() })) }
            MSG_PIECE   => { chk(&buf, 8)?; let pi = buf.get_u32(); let b = buf.get_u32(); Ok(Message::Piece { piece_index: pi, begin: b, data: buf }) }
            MSG_CANCEL  => { chk(&buf, 12)?; Ok(Message::Cancel(BlockInfo { piece_index: buf.get_u32(), begin: buf.get_u32(), length: buf.get_u32() })) }
            MSG_HASH_REQUEST => { chk(&buf, 48)?; let r = r32(&mut buf); Ok(Message::HashRequest { pieces_root: r, base_layer: buf.get_u32(), index: buf.get_u32(), length: buf.get_u32(), proof_layers: buf.get_u32() }) }
            MSG_HASHES       => { chk(&buf, 48)?; let r = r32(&mut buf); let bl = buf.get_u32(); let i = buf.get_u32(); let l = buf.get_u32(); let pl = buf.get_u32(); Ok(Message::Hashes { pieces_root: r, base_layer: bl, index: i, length: l, proof_layers: pl, hashes: buf }) }
            MSG_HASH_REJECT  => { chk(&buf, 48)?; let r = r32(&mut buf); Ok(Message::HashReject { pieces_root: r, base_layer: buf.get_u32(), index: buf.get_u32(), length: buf.get_u32(), proof_layers: buf.get_u32() }) }
            other => Err(ProtocolError::UnknownMessageId(other)),
        }
    }
}

fn chk(buf: &Bytes, n: usize) -> Result<()> {
    if buf.len() < n { Err(ProtocolError::InvalidMessageLength { expected: n, got: buf.len() }) } else { Ok(()) }
}

fn r32(buf: &mut Bytes) -> [u8; 32] {
    let mut a = [0u8; 32]; buf.copy_to_slice(&mut a); a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(msg: Message) { assert_eq!(Message::decode(msg.encode()).unwrap(), msg); }

    #[test] fn keep_alive()  { rt(Message::KeepAlive); }
    #[test] fn choke()       { rt(Message::Choke); }
    #[test] fn have()        { rt(Message::Have { piece_index: 42 }); }
    #[test] fn request()     { rt(Message::Request(BlockInfo { piece_index: 1, begin: 0, length: 16384 })); }
    #[test] fn piece()       { rt(Message::Piece { piece_index: 3, begin: 0, data: Bytes::from_static(b"data") }); }
    #[test] fn hash_request(){ rt(Message::HashRequest { pieces_root: [0xAB; 32], base_layer: 0, index: 0, length: 512, proof_layers: 2 }); }
    #[test] fn unknown_id()  { assert!(Message::decode(Bytes::from(vec![99u8])).is_err()); }
}

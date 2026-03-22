use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use crate::bucket::NodeInfo;
use crate::node_id::NodeId;
use crate::error::{DhtError, Result};

const MSG_PING:          u8 = 0;
const MSG_PONG:          u8 = 1;
const MSG_FIND_NODE:     u8 = 2;
const MSG_NODES:         u8 = 3;
const MSG_GET_PEERS:     u8 = 4;
const MSG_GOT_PEERS:     u8 = 5;
const MSG_ANNOUNCE_PEER: u8 = 6;
const MSG_ANNOUNCE_ACK:  u8 = 7;
const MSG_STORE:         u8 = 8;
const MSG_STORE_ACK:     u8 = 9;

#[derive(Debug, Clone)]
pub enum DhtMessage {
    Ping          { sender: NodeId },
    Pong          { sender: NodeId },
    FindNode      { sender: NodeId, target: NodeId },
    Nodes         { sender: NodeId, nodes: Vec<NodeInfo> },
    GetPeers      { sender: NodeId, info_hash: [u8; 32] },
    GotPeers      {
        sender:    NodeId,
        token:     [u8; 8],
        peers:     Vec<String>,   // "ip:port"
        nodes:     Vec<NodeInfo>, // más cercanos si no hay peers
    },
    AnnouncePeer  { sender: NodeId, info_hash: [u8; 32], token: [u8; 8], addr: String },
    AnnounceAck   { sender: NodeId },
    Store         { sender: NodeId, key: [u8; 32], value: Bytes, token: [u8; 8] },
    StoreAck      { sender: NodeId },
}

impl DhtMessage {
    pub fn sender(&self) -> &NodeId {
        match self {
            Self::Ping          { sender, .. } => sender,
            Self::Pong          { sender, .. } => sender,
            Self::FindNode      { sender, .. } => sender,
            Self::Nodes         { sender, .. } => sender,
            Self::GetPeers      { sender, .. } => sender,
            Self::GotPeers      { sender, .. } => sender,
            Self::AnnouncePeer  { sender, .. } => sender,
            Self::AnnounceAck   { sender, .. } => sender,
            Self::Store         { sender, .. } => sender,
            Self::StoreAck      { sender, .. } => sender,
        }
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();
        match self {
            Self::Ping { sender } => {
                buf.put_u8(MSG_PING);
                buf.put_slice(sender.as_bytes());
            }
            Self::Pong { sender } => {
                buf.put_u8(MSG_PONG);
                buf.put_slice(sender.as_bytes());
            }
            Self::FindNode { sender, target } => {
                buf.put_u8(MSG_FIND_NODE);
                buf.put_slice(sender.as_bytes());
                buf.put_slice(target.as_bytes());
            }
            Self::Nodes { sender, nodes } => {
                buf.put_u8(MSG_NODES);
                buf.put_slice(sender.as_bytes());
                encode_nodes(&mut buf, nodes);
            }
            Self::GetPeers { sender, info_hash } => {
                buf.put_u8(MSG_GET_PEERS);
                buf.put_slice(sender.as_bytes());
                buf.put_slice(info_hash);
            }
            Self::GotPeers { sender, token, peers, nodes } => {
                buf.put_u8(MSG_GOT_PEERS);
                buf.put_slice(sender.as_bytes());
                buf.put_slice(token);
                // peers
                buf.put_u16(peers.len() as u16);
                for p in peers {
                    let b = p.as_bytes();
                    buf.put_u16(b.len() as u16);
                    buf.put_slice(b);
                }
                encode_nodes(&mut buf, nodes);
            }
            Self::AnnouncePeer { sender, info_hash, token, addr } => {
                buf.put_u8(MSG_ANNOUNCE_PEER);
                buf.put_slice(sender.as_bytes());
                buf.put_slice(info_hash);
                buf.put_slice(token);
                let ab = addr.as_bytes();
                buf.put_u16(ab.len() as u16);
                buf.put_slice(ab);
            }
            Self::AnnounceAck { sender } => {
                buf.put_u8(MSG_ANNOUNCE_ACK);
                buf.put_slice(sender.as_bytes());
            }
            Self::Store { sender, key, value, token } => {
                buf.put_u8(MSG_STORE);
                buf.put_slice(sender.as_bytes());
                buf.put_slice(key);
                buf.put_slice(token);
                buf.put_u32(value.len() as u32);
                buf.put_slice(value);
            }
            Self::StoreAck { sender } => {
                buf.put_u8(MSG_STORE_ACK);
                buf.put_slice(sender.as_bytes());
            }
        }
        buf.freeze()
    }

    pub fn decode(mut buf: Bytes) -> Result<Self> {
        if buf.is_empty() {
            return Err(DhtError::InvalidNodeId(0));
        }
        let id = buf.get_u8();
        match id {
            MSG_PING => Ok(Self::Ping { sender: get_node_id(&mut buf)? }),
            MSG_PONG => Ok(Self::Pong { sender: get_node_id(&mut buf)? }),

            MSG_FIND_NODE => {
                let sender = get_node_id(&mut buf)?;
                let target = get_node_id(&mut buf)?;
                Ok(Self::FindNode { sender, target })
            }

            MSG_NODES => {
                let sender = get_node_id(&mut buf)?;
                let nodes  = decode_nodes(&mut buf)?;
                Ok(Self::Nodes { sender, nodes })
            }

            MSG_GET_PEERS => {
                let sender = get_node_id(&mut buf)?;
                let info_hash = get_arr32(&mut buf)?;
                Ok(Self::GetPeers { sender, info_hash })
            }

            MSG_GOT_PEERS => {
                let sender = get_node_id(&mut buf)?;
                let token  = get_arr8(&mut buf)?;
                let peer_count = buf.get_u16() as usize;
                let mut peers = Vec::with_capacity(peer_count);
                for _ in 0..peer_count {
                    let len = buf.get_u16() as usize;
                    let s = String::from_utf8(buf.copy_to_bytes(len).to_vec())
                        .map_err(|_| DhtError::InvalidNodeId(0))?;
                    peers.push(s);
                }
                let nodes = decode_nodes(&mut buf)?;
                Ok(Self::GotPeers { sender, token, peers, nodes })
            }

            MSG_ANNOUNCE_PEER => {
                let sender    = get_node_id(&mut buf)?;
                let info_hash = get_arr32(&mut buf)?;
                let token     = get_arr8(&mut buf)?;
                let len       = buf.get_u16() as usize;
                let addr = String::from_utf8(buf.copy_to_bytes(len).to_vec())
                    .map_err(|_| DhtError::InvalidNodeId(0))?;
                Ok(Self::AnnouncePeer { sender, info_hash, token, addr })
            }

            MSG_ANNOUNCE_ACK => Ok(Self::AnnounceAck { sender: get_node_id(&mut buf)? }),

            MSG_STORE => {
                let sender = get_node_id(&mut buf)?;
                let key    = get_arr32(&mut buf)?;
                let token  = get_arr8(&mut buf)?;
                let len    = buf.get_u32() as usize;
                let value  = buf.copy_to_bytes(len);
                Ok(Self::Store { sender, key, value, token })
            }

            MSG_STORE_ACK => Ok(Self::StoreAck { sender: get_node_id(&mut buf)? }),

            other => Err(DhtError::InvalidNodeId(other as usize)),
        }
    }
}

fn encode_nodes(buf: &mut BytesMut, nodes: &[NodeInfo]) {
    buf.put_u16(nodes.len() as u16);
    for n in nodes {
        buf.put_slice(n.id.as_bytes());
        let ab = n.addr.as_bytes();
        buf.put_u16(ab.len() as u16);
        buf.put_slice(ab);
    }
}

fn decode_nodes(buf: &mut Bytes) -> Result<Vec<NodeInfo>> {
    let count = buf.get_u16() as usize;
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        let id   = get_node_id(buf)?;
        let len  = buf.get_u16() as usize;
        let addr = String::from_utf8(buf.copy_to_bytes(len).to_vec())
            .map_err(|_| DhtError::InvalidNodeId(0))?;
        nodes.push(NodeInfo::new(id, addr));
    }
    Ok(nodes)
}

fn get_node_id(buf: &mut Bytes) -> Result<NodeId> {
    if buf.remaining() < 32 {
        return Err(DhtError::InvalidNodeId(buf.remaining()));
    }
    NodeId::from_bytes(&buf.copy_to_bytes(32))
}

fn get_arr32(buf: &mut Bytes) -> Result<[u8; 32]> {
    if buf.remaining() < 32 {
        return Err(DhtError::InvalidNodeId(buf.remaining()));
    }
    let mut arr = [0u8; 32];
    buf.copy_to_slice(&mut arr);
    Ok(arr)
}

fn get_arr8(buf: &mut Bytes) -> Result<[u8; 8]> {
    if buf.remaining() < 8 {
        return Err(DhtError::InvalidNodeId(buf.remaining()));
    }
    let mut arr = [0u8; 8];
    buf.copy_to_slice(&mut arr);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(msg: DhtMessage) {
        let enc = msg.encode();
        DhtMessage::decode(enc).unwrap();
    }

    #[test]
    fn ping_pong()       { rt(DhtMessage::Ping { sender: NodeId::random() }); rt(DhtMessage::Pong { sender: NodeId::random() }); }
    #[test]
    fn find_node()       { rt(DhtMessage::FindNode { sender: NodeId::random(), target: NodeId::random() }); }
    #[test]
    fn nodes()           { rt(DhtMessage::Nodes { sender: NodeId::random(), nodes: vec![NodeInfo::new(NodeId::random(), "1.2.3.4:6881".into())] }); }
    #[test]
    fn get_peers()       { rt(DhtMessage::GetPeers { sender: NodeId::random(), info_hash: [0xAB; 32] }); }
    #[test]
    fn got_peers()       {
        rt(DhtMessage::GotPeers {
            sender: NodeId::random(), token: [1u8; 8],
            peers: vec!["1.2.3.4:6881".into()],
            nodes: vec![],
        });
    }
    #[test]
    fn announce_peer()   { rt(DhtMessage::AnnouncePeer { sender: NodeId::random(), info_hash: [0u8; 32], token: [2u8; 8], addr: "1.2.3.4:6881".into() }); }
    #[test]
    fn announce_ack()    { rt(DhtMessage::AnnounceAck { sender: NodeId::random() }); }
    #[test]
    fn store()           { rt(DhtMessage::Store { sender: NodeId::random(), key: [0u8; 32], value: Bytes::from_static(b"hello"), token: [3u8; 8] }); }
    #[test]
    fn store_ack()       { rt(DhtMessage::StoreAck { sender: NodeId::random() }); }
}

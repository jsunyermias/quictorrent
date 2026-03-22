use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use qt_protocol::{BlockInfo, Handshake, Message};
use qt_transport::PeerConnection;

use crate::{
    error::{PeerError, Result},
    state::{PendingRequest, SessionState},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MSG_TIMEOUT: Duration = Duration::from_secs(120);

pub trait SessionHandler: Send + 'static {
    fn on_piece(&mut self, piece_index: u32, begin: u32, data: bytes::Bytes);
    fn on_have(&mut self, piece_index: u32);
    fn on_bitfield(&mut self, bitfield: bytes::Bytes);
    fn on_request(&mut self, piece_index: u32, begin: u32, length: u32) -> Option<bytes::Bytes>;
}

pub struct PeerSession {
    conn: PeerConnection,
    pub state: SessionState,
    expected_info_hash: [u8; 20],
    our_peer_id: [u8; 20],
}

impl PeerSession {
    pub fn new(
        conn: PeerConnection,
        num_pieces: usize,
        expected_info_hash: [u8; 20],
        our_peer_id: [u8; 20],
    ) -> Self {
        Self {
            conn,
            state: SessionState::new(num_pieces),
            expected_info_hash,
            our_peer_id,
        }
    }

    pub async fn handshake_outbound(&self) -> Result<[u8; 20]> {
        let (send, recv) = self.conn.inner_conn().open_bi().await
            .map_err(qt_transport::TransportError::Connection)?;
        self.do_handshake(send, recv).await
    }

    pub async fn handshake_inbound(&self) -> Result<[u8; 20]> {
        let (send, recv) = self.conn.inner_conn().accept_bi().await
            .map_err(qt_transport::TransportError::Connection)?;
        self.do_handshake(send, recv).await
    }

    async fn do_handshake(
        &self,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) -> Result<[u8; 20]> {
        let our_hs = Handshake::new(self.expected_info_hash, self.our_peer_id, [0u8; 8]);

        timeout(HANDSHAKE_TIMEOUT, send.write_all(&our_hs.encode()))
            .await.map_err(|_| PeerError::Timeout)??;

        let mut buf = vec![0u8; Handshake::LEN];
        timeout(HANDSHAKE_TIMEOUT, recv.read_exact(&mut buf))
            .await.map_err(|_| PeerError::Timeout)??;

        let their_hs = Handshake::decode(bytes::Bytes::from(buf))?;

        if their_hs.info_hash != self.expected_info_hash {
            return Err(PeerError::InfoHashMismatch);
        }

        info!(peer = %self.conn.remote_addr(), peer_id = %hex(&their_hs.peer_id), "handshake ok");
        Ok(their_hs.peer_id)
    }

    pub async fn run<H: SessionHandler>(mut self, mut handler: H) -> Result<()> {
        let (mut writer, mut reader) = self.conn.open_bidi_stream().await?;

        loop {
            let msg = match timeout(MSG_TIMEOUT, reader.next()).await {
                Err(_)           => return Err(PeerError::Timeout),
                Ok(None)         => return Err(PeerError::Disconnected),
                Ok(Some(Err(e))) => return Err(PeerError::Protocol(e)),
                Ok(Some(Ok(m)))  => m,
            };

            debug!(peer = %self.conn.remote_addr(), ?msg, "received");

            match msg {
                Message::KeepAlive       => {}
                Message::Choke           => { self.state.apply_choke(); }
                Message::Unchoke         => { self.state.apply_unchoke(); }
                Message::Interested      => { self.state.apply_interested(); }
                Message::NotInterested   => { self.state.apply_not_interested(); }

                Message::Have { piece_index } => {
                    self.state.apply_have(piece_index);
                    handler.on_have(piece_index);
                }

                Message::Bitfield(bits) => {
                    self.state.apply_bitfield(&bits);
                    handler.on_bitfield(bits);
                }

                Message::Request(block) => {
                    if self.state.local_to_remote.choked {
                        warn!("ignoring request from choked peer");
                        continue;
                    }
                    if let Some(data) = handler.on_request(block.piece_index, block.begin, block.length) {
                        writer.send(Message::Piece {
                            piece_index: block.piece_index,
                            begin: block.begin,
                            data,
                        }).await?;
                    }
                }

                Message::Piece { piece_index, begin, data } => {
                    self.state.remove_pending_request(piece_index, begin);
                    handler.on_piece(piece_index, begin, data);
                }

                Message::Cancel(block) => {
                    self.state.remove_pending_request(block.piece_index, block.begin);
                }

                Message::HashRequest { .. }
                | Message::Hashes { .. }
                | Message::HashReject { .. } => {
                    debug!("BEP-52 message (not yet handled)");
                }
            }
        }
    }

    pub async fn send_interested(&self) -> Result<()> {
        self.conn.send_message(Message::Interested).await?;
        Ok(())
    }

    pub async fn send_unchoke(&self) -> Result<()> {
        self.conn.send_message(Message::Unchoke).await?;
        Ok(())
    }

    pub async fn request_block(&mut self, piece_index: u32, begin: u32, length: u32) -> Result<()> {
        if !self.state.can_request() {
            return Err(PeerError::HandshakeFailed("choked or not interested".into()));
        }
        self.state.add_pending_request(PendingRequest { piece_index, begin, length });
        self.conn.send_message(Message::Request(BlockInfo { piece_index, begin, length })).await?;
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

use quinn::{Connection, RecvStream, SendStream};
use tokio_util::codec::{FramedRead, FramedWrite};
use futures_util::SinkExt;

use bitturbulence_protocol::{Message, MessageCodec};
use crate::error::Result;

#[derive(Clone, Debug)]
pub struct PeerConnection {
    inner: Connection,
}

impl PeerConnection {
    pub fn new(conn: Connection) -> Self {
        Self { inner: conn }
    }

    pub fn remote_addr(&self) -> std::net::SocketAddr {
        self.inner.remote_address()
    }

    pub async fn open_bidi_stream(
        &self,
    ) -> Result<(
        FramedWrite<SendStream, MessageCodec>,
        FramedRead<RecvStream, MessageCodec>,
    )> {
        let (send, recv) = self.inner.open_bi().await?;
        Ok((
            FramedWrite::new(send, MessageCodec),
            FramedRead::new(recv, MessageCodec),
        ))
    }

    pub async fn accept_bidi_stream(
        &self,
    ) -> Result<(
        FramedWrite<SendStream, MessageCodec>,
        FramedRead<RecvStream, MessageCodec>,
    )> {
        let (send, recv) = self.inner.accept_bi().await?;
        Ok((
            FramedWrite::new(send, MessageCodec),
            FramedRead::new(recv, MessageCodec),
        ))
    }

    pub async fn send_message(&self, msg: Message) -> Result<()> {
        let (send, recv) = self.inner.open_bi().await?;
        let mut writer: FramedWrite<SendStream, MessageCodec> =
            FramedWrite::new(send, MessageCodec);
        writer.send(msg).await?;
        writer.close().await?;
        drop(recv);
        Ok(())
    }

    pub fn close(&self, code: u32, reason: &[u8]) {
        self.inner.close(quinn::VarInt::from_u32(code), reason);
    }

    pub fn stats(&self) -> quinn::ConnectionStats {
        self.inner.stats()
    }
}

impl PeerConnection {
    /// Acceso al quinn::Connection subyacente para casos avanzados.
    /// Usar con precaución — preferir los métodos de alto nivel.
    pub fn inner_conn(&self) -> &quinn::Connection {
        &self.inner
    }
}

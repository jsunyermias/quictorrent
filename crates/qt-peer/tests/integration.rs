use std::net::SocketAddr;
use bytes::Bytes;
use qt_peer::{PeerSession, SessionHandler};
use qt_transport::QuicEndpoint;

const INFO_HASH: [u8; 20] = [0xAB; 20];
const PEER_ID_A: [u8; 20] = [0x41; 20];
const PEER_ID_B: [u8; 20] = [0x42; 20];
const NUM_PIECES: usize = 10;

struct TestHandler;

impl SessionHandler for TestHandler {
    fn on_piece(&mut self, _: u32, _: u32, _: Bytes) {}
    fn on_have(&mut self, _: u32) {}
    fn on_bitfield(&mut self, _: Bytes) {}
    fn on_request(&mut self, _: u32, _: u32, _: u32) -> Option<Bytes> { None }
}

#[tokio::test]
async fn handshake_succeeds() {
    let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let server_ep = QuicEndpoint::bind(server_addr).unwrap();
    let server_addr = server_ep.local_addr().unwrap();

    // Canal para que servidor comunique el peer_id al test principal
    let (tx, rx) = tokio::sync::oneshot::channel::<[u8; 20]>();

    let server_task = tokio::spawn(async move {
        let conn = server_ep.accept().await.unwrap().unwrap();
        let session = PeerSession::new(conn, NUM_PIECES, INFO_HASH, PEER_ID_B);
        let peer_id = session.handshake_inbound().await.unwrap();
        tx.send(peer_id).unwrap();
        // Mantener el endpoint vivo hasta que el test termine.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });

    let client_ep = QuicEndpoint::client_only().unwrap();
    let conn = client_ep.connect(server_addr).await.unwrap();
    let session = PeerSession::new(conn, NUM_PIECES, INFO_HASH, PEER_ID_A);
    let peer_id = session.handshake_outbound().await.unwrap();

    assert_eq!(peer_id, PEER_ID_B);

    let server_peer_id = rx.await.unwrap();
    assert_eq!(server_peer_id, PEER_ID_A);

    server_task.abort();
}

#[tokio::test]
async fn wrong_info_hash_rejected() {
    let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let server_ep = QuicEndpoint::bind(server_addr).unwrap();
    let server_addr = server_ep.local_addr().unwrap();

    let _server_task = tokio::spawn(async move {
        let conn = server_ep.accept().await.unwrap().unwrap();
        let session = PeerSession::new(conn, NUM_PIECES, INFO_HASH, PEER_ID_B);
        let _ = session.handshake_inbound().await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });

    let client_ep = QuicEndpoint::client_only().unwrap();
    let conn = client_ep.connect(server_addr).await.unwrap();
    let session = PeerSession::new(conn, NUM_PIECES, [0xFF; 20], PEER_ID_A);
    let result = session.handshake_outbound().await;

    assert!(result.is_err());
}

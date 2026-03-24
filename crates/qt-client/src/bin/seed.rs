//! Seeder mínimo: sirve un archivo a quien conecte.
//! Uso: cargo run --bin seed -- <archivo> <puerto>

use std::net::SocketAddr;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use qt_pieces::{hash_block, piece_root, file_root};
use qt_protocol::{Message, MessageCodec};
use qt_transport::QuicEndpoint;
use tokio_util::codec::{FramedRead, FramedWrite};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("uso: seed <archivo> <puerto>");
        std::process::exit(1);
    }

    let file_path = &args[1];
    let port: u16  = args[2].parse()?;

    // Leer el archivo completo
    let data = std::fs::read(file_path)?;
    let file_size = data.len() as u64;
    let piece_length = qt_protocol::piece_length_for_size(file_size) as usize;

    // Calcular raíces Merkle de piezas (árbol bloque → pieza → archivo)
    let pieces: Vec<Vec<u8>> = data.chunks(piece_length).map(|c| c.to_vec()).collect();
    let num_pieces = pieces.len();
    // piece_root = raíz Merkle de hashes SHA-256 de bloques de 16 KiB
    let hashes: Vec<[u8; 32]> = pieces.iter().map(|p| piece_root(p)).collect();
    // file_root = raíz Merkle de piece_roots (relleno a potencia de 2)
    let f_root = file_root(&hashes);
    // info_hash = SHA-256(file_root) — para torrent multi-file: SHA-256(root_0 || root_1 || ...)
    let info_hash = hash_block(&f_root);

    println!("archivo:      {}", file_path);
    println!("tamaño:       {} bytes", file_size);
    println!("piece_length: {} bytes", piece_length);
    println!("num_pieces:   {}", num_pieces);
    println!("info_hash:    {}", hex::encode(info_hash));
    println!("escuchando en 0.0.0.0:{}", port);
    println!("---");

    let bind_addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let endpoint  = QuicEndpoint::bind(bind_addr)?;

    loop {
        let Some(Ok(conn)) = endpoint.accept().await else { continue };
        let peer_addr = conn.remote_addr();
        println!("[{}] conexión entrante", peer_addr);

        let pieces_clone = pieces.clone();
        let info_hash_clone = info_hash;

        tokio::spawn(async move {
            let (send, recv) = conn.inner_conn().accept_bi().await.unwrap();
            let mut writer = FramedWrite::new(send, MessageCodec);
            let mut reader = FramedRead::new(recv, MessageCodec);

            // Esperar KeepAlive inicial
            loop {
                match reader.next().await.unwrap().unwrap() {
                    Message::KeepAlive => continue,
                    Message::Hello { peer_id, info_hash, .. } => {
                        if info_hash != info_hash_clone {
                            println!("[{}] info_hash incorrecto", peer_addr);
                            writer.send(Message::HelloAck {
                                peer_id: [0u8; 32],
                                accepted: false,
                                reason: Some("info_hash mismatch".into()),
                            }).await.unwrap();
                            return;
                        }
                        println!("[{}] hello ok, peer_id={}", peer_addr, hex::encode(&peer_id[..4]));
                        writer.send(Message::HelloAck {
                            peer_id: [0x53u8; 32], // 'S' de Seeder
                            accepted: true,
                            reason: None,
                        }).await.unwrap();
                        break;
                    }
                    m => { println!("[{}] mensaje inesperado: {:?}", peer_addr, m); return; }
                }
            }

            // Anunciar que tenemos todo
            writer.send(Message::HaveAll { file_index: 0 }).await.unwrap();

            // Servir requests
            loop {
                match reader.next().await {
                    None | Some(Err(_)) => { println!("[{}] desconectado", peer_addr); break; }
                    Some(Ok(Message::Request { file_index: 0, piece_index, begin, length })) => {
                        let pi = piece_index as usize;
                        if pi >= pieces_clone.len() {
                            writer.send(Message::Reject {
                                file_index: 0, piece_index, begin, length,
                            }).await.unwrap();
                            continue;
                        }
                        let piece = &pieces_clone[pi];
                        let begin_u = begin as usize;
                        let end_u = (begin + length) as usize;
                        let data = Bytes::copy_from_slice(&piece[begin_u..end_u.min(piece.len())]);
                        writer.send(Message::Piece {
                            file_index: 0, piece_index, begin, data,
                        }).await.unwrap();
                        println!("[{}] pieza {}/{} enviada", peer_addr, piece_index + 1, pieces_clone.len());
                    }
                    Some(Ok(Message::Bye { reason })) => {
                        println!("[{}] bye: {}", peer_addr, reason);
                        break;
                    }
                    Some(Ok(_)) => {}
                }
            }
        });
    }
}

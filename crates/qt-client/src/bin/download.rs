//! Descargador mínimo: descarga un archivo de un seeder.
//! Uso: cargo run --bin download -- <ip:puerto> <info_hash_hex> <num_pieces> <piece_length> <output>

use std::net::SocketAddr;
use futures_util::{SinkExt, StreamExt};
use qt_pieces::hash_piece;
use qt_protocol::{AuthPayload, Message, MessageCodec, PROTOCOL_VERSION};
use qt_transport::QuicEndpoint;
use tokio_util::codec::{FramedRead, FramedWrite};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!("uso: download <ip:puerto> <info_hash_hex> <num_pieces> <piece_length> <output>");
        std::process::exit(1);
    }

    let addr: SocketAddr    = args[1].parse()?;
    let info_hash_hex       = &args[2];
    let num_pieces: usize   = args[3].parse()?;
    let piece_length: usize = args[4].parse()?;
    let output_path         = &args[5];

    let info_hash_bytes = hex::decode(info_hash_hex)?;
    let mut info_hash = [0u8; 32];
    info_hash.copy_from_slice(&info_hash_bytes);

    println!("conectando a {} ...", addr);

    let endpoint = QuicEndpoint::client_only()?;
    let conn = endpoint.connect(addr).await?;

    let (send, recv) = conn.inner_conn().open_bi().await?;
    let mut writer = FramedWrite::new(send, MessageCodec);
    let mut reader = FramedRead::new(recv, MessageCodec);

    // Materializar stream
    writer.send(Message::KeepAlive).await?;

    // Hello
    writer.send(Message::Hello {
        version:   PROTOCOL_VERSION,
        peer_id:   [0x44u8; 32], // 'D' de Downloader
        info_hash,
        auth:      AuthPayload::None,
    }).await?;

    // Esperar HelloAck
    loop {
        match reader.next().await.unwrap()? {
            Message::KeepAlive => continue,
            Message::HelloAck { accepted, reason, .. } => {
                if !accepted {
                    anyhow::bail!("rechazado: {}", reason.unwrap_or_default());
                }
                println!("hello aceptado");
                break;
            }
            m => println!("ignorando: {:?}", m),
        }
    }

    // Esperar HaveAll
    loop {
        match reader.next().await.unwrap()? {
            Message::KeepAlive => continue,
            Message::HaveAll { .. } => { println!("seeder tiene todo"); break; }
            m => println!("ignorando: {:?}", m),
        }
    }

    // Descargar piezas secuencialmente
    let mut pieces: Vec<Option<Vec<u8>>> = vec![None; num_pieces];

    for (pi, slot) in pieces.iter_mut().enumerate() {
        // La última pieza puede ser más corta — pedimos piece_length y luego truncamos
        let length = piece_length as u32;

        writer.send(Message::Request {
            file_index:  0,
            piece_index: pi as u32,
            begin:       0,
            length,
        }).await?;

        loop {
            match reader.next().await.unwrap()? {
                Message::KeepAlive => continue,
                Message::Piece { piece_index, begin: 0, data, .. } if piece_index == pi as u32 => {
                    *slot = Some(data.to_vec());
                    print!("\rpieza {}/{} descargada", pi + 1, num_pieces);
                    std::io::Write::flush(&mut std::io::stdout())?;
                    break;
                }
                Message::Reject { piece_index, .. } => {
                    anyhow::bail!("pieza {} rechazada", piece_index);
                }
                _ => continue,
            }
        }
    }

    println!("\ntodas las piezas recibidas, ensamblando...");

    // Ensamblar y escribir
    let mut output = Vec::new();
    for p in pieces.into_iter().flatten() {
        output.extend_from_slice(&p);
    }

    std::fs::write(output_path, &output)?;
    println!("guardado en {}", output_path);
    println!("tamaño: {} bytes", output.len());

    // Verificar integridad
    let chunks: Vec<&[u8]> = output.chunks(piece_length).collect();
    let mut all_hashes = Vec::new();
    for chunk in &chunks {
        all_hashes.extend_from_slice(&hash_piece(chunk));
    }
    let computed_hash = hash_piece(&all_hashes);
    if computed_hash == info_hash {
        println!("✅ verificación SHA-256 correcta");
    } else {
        println!("❌ verificación SHA-256 FALLÓ");
        println!("   esperado:  {}", info_hash_hex);
        println!("   calculado: {}", hex::encode(computed_hash));
    }

    writer.send(Message::Bye { reason: "done".into() }).await?;
    Ok(())
}

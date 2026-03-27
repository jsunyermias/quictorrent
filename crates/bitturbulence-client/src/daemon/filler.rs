use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::time::interval;
use tracing::{debug, info, Instrument, Span};

use bitturbulence_protocol::Message;
use bitturbulence_transport::PeerConnection;

use super::context::FlowCtx;
use super::drainer::send_our_bitfields;
use super::stream::{serve_data_stream, CancelSet};
use super::KEEPALIVE_INTERVAL;

// ── Bucle del filler (conexión entrante) ──────────────────────────────────────

pub async fn run_peer_filler(
    conn: &PeerConnection,
    ctx: &Arc<FlowCtx>,
    _peer_id: &[u8; 32],
) -> Result<()> {
    // Stream 1 ya fue gestionado por handle_inbound (Hello / HelloAck).
    // Aceptamos el stream 2 como canal de control.
    let (mut ctrl_w, mut ctrl_r) = conn.accept_bidi_stream().await?;

    // Conjunto compartido de bloques cancelados: el drainer envía Cancel en el
    // canal de control; cada serve_data_stream lo comprueba antes de enviar Piece.
    let cancel_set: CancelSet = Arc::new(Mutex::new(HashSet::new()));

    // Anunciar nuestra disponibilidad al drainer.
    send_our_bitfields(ctx, &mut ctrl_w).await?;

    let mut have_piece_rx = ctx.have_piece_tx.subscribe();
    let mut ka_timer = interval(KEEPALIVE_INTERVAL);
    ka_timer.tick().await;

    loop {
        tokio::select! {
            msg_opt = ctrl_r.next() => {
                match msg_opt {
                    Some(Ok(Message::KeepAlive))          => {}
                    Some(Ok(Message::HavePiece { .. }))   => {}
                    Some(Ok(Message::HaveNone  { .. }))   => {}
                    Some(Ok(Message::HaveAll   { .. }))   => {}
                    Some(Ok(Message::HaveBitmap { .. }))  => {}
                    Some(Ok(Message::Cancel {
                        file_index,
                        piece_index,
                        begin,
                        ..
                    })) => {
                        cancel_set
                            .lock()
                            .unwrap()
                            .insert((file_index, piece_index, begin));
                    }
                    Some(Ok(Message::Bye { reason }))     => {
                        info!("peer bye: {reason}");
                        return Ok(());
                    }
                    Some(Ok(_))  => {}
                    Some(Err(e)) => return Err(e.into()),
                    None         => return Ok(()),
                }
            }

            // Aceptar nuevos data streams del drainer.
            stream = conn.accept_bidi_stream() => {
                match stream {
                    Ok((w, r)) => {
                        tokio::spawn(
                            serve_data_stream(w, r, ctx.clone(), cancel_set.clone())
                                .instrument(Span::current()),
                        );
                    }
                    Err(e) => {
                        tracing::debug!("accept data stream: {e}");
                        break;
                    }
                }
            }

            piece = have_piece_rx.recv() => {
                match piece {
                    Ok((fi, pi)) => {
                        let _ = ctrl_w.send(Message::HavePiece {
                            file_index: fi as u16,
                            piece_index: pi,
                        }).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!("have_piece_rx lagged by {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }

            _ = ka_timer.tick() => {
                ctrl_w.send(Message::KeepAlive).await?;
            }
        }
    }

    Ok(())
}

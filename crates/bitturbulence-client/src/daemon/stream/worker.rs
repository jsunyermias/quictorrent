use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, warn};

use bitturbulence_pieces::{hash_block, piece_root_from_block_hashes, BlockTask};
use bitturbulence_protocol::Message;

use super::super::context::FlowCtx;
use super::super::{HASH_RESPONSE_TIMEOUT, SNUB_TIMEOUT};
use super::types::{R, StreamResult, W};

/// Descarga repetidamente bloques asignados por el coordinador sobre un
/// stream QUIC dedicado.
///
/// ## Verificación Merkle incremental (#32)
///
/// Al cambiar de pieza, envía un `HashRequest` al filler y espera
/// `HashResponse` con los hashes SHA-256 de cada bloque. Verifica que su
/// raíz Merkle coincide con el hash de pieza del metainfo antes de aceptar
/// ningún dato. Si la raíz no coincide (filler mintiendo), cada bloque de esa
/// pieza se marca fallido de inmediato. Si el filler responde vacío (no tiene
/// la pieza), se omite la verificación por bloque y se delega en la
/// verificación de pieza al final.
pub(crate) async fn download_stream_worker(
    stream_id: usize,
    mut writer: W,
    mut reader: R,
    mut task_rx: mpsc::Receiver<BlockTask>,
    result_tx: mpsc::Sender<StreamResult>,
    ctx: Arc<FlowCtx>,
) {
    // Caché de hashes de bloque de la pieza que descargamos actualmente.
    // Se invalida al cambiar de (fi, pi).
    let mut curr_piece: Option<(usize, u32)> = None;
    let mut curr_block_hashes: Vec<[u8; 32]> = Vec::new();

    while let Some(BlockTask {
        fi,
        pi,
        bi,
        begin,
        length,
    }) = task_rx.recv().await
    {
        // ── Solicitar hashes de bloque si es una pieza nueva ────────────
        if curr_piece != Some((fi, pi)) {
            if writer
                .send(Message::HashRequest {
                    file_index: fi as u16,
                    piece_index: pi,
                })
                .await
                .is_err()
            {
                let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
                return;
            }

            // Esperar HashResponse con timeout para evitar que el stream
            // worker quede bloqueado indefinidamente ante un filler lento o buggy.
            // Devuelve Some(hashes) en éxito/mismatch, None si el stream murió.
            let hash_result = timeout(HASH_RESPONSE_TIMEOUT, async {
                loop {
                    match reader.next().await {
                        Some(Ok(Message::HashResponse {
                            file_index,
                            piece_index,
                            block_hashes,
                        })) if file_index as usize == fi && piece_index == pi => {
                            if !block_hashes.is_empty() {
                                let computed = piece_root_from_block_hashes(&block_hashes);
                                let Some(&expected) =
                                    ctx.meta.files[fi].piece_hashes.get(pi as usize)
                                else {
                                    warn!(fi, pi, "piece_index fuera de rango en metainfo");
                                    return Some(vec![]);
                                };
                                if computed != expected {
                                    warn!(
                                        fi,
                                        pi,
                                        "HashResponse: raíz Merkle incorrecta \
                                                  — filler puede estar sirviendo datos corruptos"
                                    );
                                    return Some(vec![]);
                                }
                            }
                            return Some(block_hashes);
                        }
                        Some(Ok(Message::HashResponse { .. })) => {} // otra pieza, ignorar
                        None | Some(Err(_)) => return None, // stream muerto
                        Some(Ok(_)) => {}
                    }
                }
            })
            .await;

            match hash_result {
                Ok(Some(hashes)) => curr_block_hashes = hashes,
                Ok(None) => {
                    // Stream cerrado mientras esperábamos HashResponse.
                    let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
                    return;
                }
                Err(_elapsed) => {
                    warn!(fi, pi, "timeout esperando HashResponse — stream dead");
                    let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
                    return;
                }
            }
            curr_piece = Some((fi, pi));
        }

        // ── Enviar Request ────────────────────────────────────────────────
        if writer
            .send(Message::Request {
                file_index: fi as u16,
                piece_index: pi,
                begin,
                length,
            })
            .await
            .is_err()
        {
            let _ = result_tx.send(StreamResult::StreamDead { stream_id }).await;
            return;
        }

        // Esperar Piece con timeout de snub: si el filler no responde en
        // SNUB_TIMEOUT, el coordinador marcará este peer como snubbed.
        let result = match timeout(SNUB_TIMEOUT, async {
            loop {
                match reader.next().await {
                    Some(Ok(Message::Piece {
                        file_index,
                        piece_index,
                        begin: b,
                        data,
                    })) if file_index as usize == fi && piece_index == pi && b == begin => {
                        let block_hash = hash_block(&data);

                        // Verificación incremental: comparar con hash esperado.
                        if let Some(&expected_hash) = curr_block_hashes.get(bi as usize) {
                            if block_hash != expected_hash {
                                warn!(
                                    fi,
                                    pi,
                                    bi,
                                    "hash de bloque incorrecto — \
                                                  datos corruptos del filler"
                                );
                                return StreamResult::BlockFail {
                                    stream_id,
                                    fi,
                                    pi,
                                    bi,
                                };
                            }
                        }

                        return match ctx.store.write_block(fi, pi, begin, &data).await {
                            Ok(()) => StreamResult::BlockOk {
                                stream_id,
                                fi,
                                pi,
                                bi,
                                hash: block_hash,
                            },
                            Err(e) => {
                                warn!(fi, pi, bi, "write_block: {e}");
                                StreamResult::BlockFail {
                                    stream_id,
                                    fi,
                                    pi,
                                    bi,
                                }
                            }
                        };
                    }
                    Some(Ok(Message::Reject {
                        file_index,
                        piece_index,
                        begin: b,
                        ..
                    })) if file_index as usize == fi && piece_index == pi && b == begin => {
                        debug!(fi, pi, bi, "block rejected by filler");
                        return StreamResult::BlockFail {
                            stream_id,
                            fi,
                            pi,
                            bi,
                        };
                    }
                    None | Some(Err(_)) => {
                        // Retornamos un centinela especial para indicar stream dead.
                        return StreamResult::StreamDead { stream_id };
                    }
                    Some(Ok(_)) => {}
                }
            }
        })
        .await
        {
            Ok(r) => r,
            Err(_elapsed) => {
                warn!(fi, pi, bi, "snub timeout: filler no respondió en SNUB_TIMEOUT");
                StreamResult::BlockTimeout {
                    stream_id,
                    fi,
                    pi,
                    bi,
                }
            }
        };

        // StreamDead dentro del timeout → propagar y salir.
        if matches!(result, StreamResult::StreamDead { .. }) {
            let _ = result_tx.send(result).await;
            return;
        }
        let _ = result_tx.send(result).await;
    }
}

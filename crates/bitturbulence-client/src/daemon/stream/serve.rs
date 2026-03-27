use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tracing::{debug, warn};

use bitturbulence_pieces::hash_block;
use bitturbulence_protocol::{Message, BLOCK_SIZE};

use super::super::context::FlowCtx;
use super::types::{CancelSet, R, W};

/// Procesa `Request`s y `HashRequest`s del drainer sobre un stream de datos
/// dedicado. Responde con `Piece`/`Reject` y `HashResponse` respectivamente.
///
/// `cancel_set` es compartido con el bucle de control del filler: si el drainer
/// envía `Cancel` para un bloque antes de que lo sirvamos, respondemos `Reject`
/// en lugar de `Piece` para evitar transmitir datos innecesarios.
pub(crate) async fn serve_data_stream(
    mut writer: W,
    mut reader: R,
    ctx: Arc<FlowCtx>,
    cancel_set: CancelSet,
) {
    while let Some(msg) = reader.next().await {
        match msg {
            Ok(Message::Request {
                file_index,
                piece_index,
                begin,
                length,
            }) => {
                // Comprobar si el drainer ya canceló este bloque.
                let cancelled = cancel_set
                    .lock()
                    .unwrap()
                    .remove(&(file_index, piece_index, begin));
                if cancelled {
                    debug!(
                        fi = file_index,
                        pi = piece_index,
                        begin,
                        "bloque cancelado — enviando Reject"
                    );
                    let _ = writer
                        .send(Message::Reject {
                            file_index,
                            piece_index,
                            begin,
                            length,
                        })
                        .await;
                    continue;
                }

                let fi = file_index as usize;
                let have = ctx.our_bitfield(fi).await;
                if have.get(piece_index as usize).copied().unwrap_or(false) {
                    match ctx.store.read_block(fi, piece_index, begin, length).await {
                        Ok(data) => {
                            // Segunda comprobación: puede haber llegado el Cancel
                            // mientras leíamos el bloque del disco.
                            let cancelled = cancel_set
                                .lock()
                                .unwrap()
                                .remove(&(file_index, piece_index, begin));
                            if cancelled {
                                debug!(
                                    fi = file_index,
                                    pi = piece_index,
                                    begin,
                                    "bloque cancelado tras lectura — enviando Reject"
                                );
                                let _ = writer
                                    .send(Message::Reject {
                                        file_index,
                                        piece_index,
                                        begin,
                                        length,
                                    })
                                    .await;
                            } else {
                                let _ = writer
                                    .send(Message::Piece {
                                        file_index,
                                        piece_index,
                                        begin,
                                        data: Bytes::from(data),
                                    })
                                    .await;
                            }
                        }
                        Err(e) => {
                            warn!(fi, piece_index, begin, "read_block: {e}");
                            let _ = writer
                                .send(Message::Reject {
                                    file_index,
                                    piece_index,
                                    begin,
                                    length,
                                })
                                .await;
                        }
                    }
                } else {
                    let _ = writer
                        .send(Message::Reject {
                            file_index,
                            piece_index,
                            begin,
                            length,
                        })
                        .await;
                }
            }

            // Verificación Merkle incremental (#32): el drainer pide los hashes
            // de bloque de una pieza antes de descargarla para verificar cada
            // bloque individualmente a medida que llega.
            Ok(Message::HashRequest {
                file_index,
                piece_index,
            }) => {
                let fi = file_index as usize;
                let pi = piece_index;
                let have = ctx.our_bitfield(fi).await;
                let block_hashes = if have.get(pi as usize).copied().unwrap_or(false) {
                    let piece_len = ctx.meta.files[fi].piece_len(pi);
                    let num_blocks = piece_len.div_ceil(BLOCK_SIZE);
                    let mut hashes = Vec::with_capacity(num_blocks as usize);
                    let mut all_ok = true;
                    for bi in 0..num_blocks {
                        let block_begin = bi * BLOCK_SIZE;
                        let block_len = (piece_len - block_begin).min(BLOCK_SIZE);
                        match ctx.store.read_block(fi, pi, block_begin, block_len).await {
                            Ok(data) => hashes.push(hash_block(&data)),
                            Err(e) => {
                                warn!(fi, pi, bi, "read_block for HashResponse: {e}");
                                all_ok = false;
                                break;
                            }
                        }
                    }
                    if all_ok {
                        hashes
                    } else {
                        vec![]
                    }
                } else {
                    vec![] // No tenemos la pieza: respuesta vacía
                };
                let _ = writer
                    .send(Message::HashResponse {
                        file_index,
                        piece_index,
                        block_hashes,
                    })
                    .await;
            }

            Ok(_) => {}
            Err(e) => {
                debug!("data stream error: {e}");
                break;
            }
        }
    }
}

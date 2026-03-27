//! Scheduler de bloques con multiplexación QUIC.
//!
//! ## Modelo
//!
//! Un **bloque** (16 KiB) es la unidad atómica de Request/Piece en el wire.
//! Una **pieza** es un grupo de bloques y es la unidad de verificación SHA-256.
//!
//! Cada bloque puede estar siendo descargado por hasta [`types::MAX_STREAMS_PER_BLOCK`]
//! streams simultáneos. El scheduler interno prioriza dentro del archivo:
//!
//! 1. Bloques `Pending` en piezas rarest-first.
//! 2. Bloques `InFlight(n < TYPICAL)` — redundancia.
//! 3. Bloques `InFlight(n < MAX)` — endgame.
//!
//! La selección entre archivos (prioridad de usuario × rareza) se aplica en el
//! coordinador del drainer, que usa [`BlockScheduler::priority`] y
//! [`BlockScheduler::min_availability`] para decidir de qué archivo descargar.
//!
//! ## Módulos
//!
//! | Módulo          | Responsabilidad                              |
//! |-----------------|----------------------------------------------|
//! | `types`         | `BlockState`, `BlockTask`, constantes        |
//! | `availability`  | Gestión de bitfields de peers                |
//! | `blocks`        | Estado de bloques/piezas y consultas         |
//! | `pick`          | Algoritmo de scheduling rarest-first         |

mod availability;
mod blocks;
mod pick;
mod tests;
pub mod types;

pub use types::{BlockState, BlockTask, MAX_STREAMS_PER_BLOCK, TYPICAL_STREAMS_PER_BLOCK};

use bitturbulence_protocol::{Priority, BLOCK_SIZE};

/// Scheduler de descarga a nivel de bloque para un único archivo.
///
/// Gestiona el estado de cada bloque de cada pieza y expone métodos para
/// asignar trabajo a streams QUIC respetando los límites de multiplexación.
/// La prioridad del archivo (respecto al resto de archivos del BitFlow) se
/// configura con [`set_priority`] y es consultada por el drainer.
pub struct BlockScheduler {
    pub(super) fi: usize,
    pub(super) piece_length: u32,
    pub(super) last_piece_len: u32,
    pub(super) num_pieces: usize,

    /// Estado por bloque: `block_state[pi][bi]`.
    pub(super) block_state: Vec<Vec<BlockState>>,

    /// Hash SHA-256 de cada bloque recibido: `block_hashes[pi][bi]`.
    pub(super) block_hashes: Vec<Vec<Option<[u8; 32]>>>,

    /// Pieza marcada como totalmente descargada y con hash verificado.
    pub(super) piece_done: Vec<bool>,

    /// Pieza en proceso de verificación (evita doble verificación).
    pub(super) piece_verifying: Vec<bool>,

    /// Cuántos peers tienen cada pieza (rarest-first).
    pub(super) availability: Vec<u32>,

    /// Prioridad de descarga del archivo completo, asignada por el usuario.
    priority: Priority,
}

impl BlockScheduler {
    /// Crea un scheduler para el archivo `fi` con prioridad `Normal`.
    ///
    /// `piece_length` debe ser múltiplo de [`BLOCK_SIZE`].
    /// `last_piece_len` es la longitud real de la última pieza.
    pub fn new(fi: usize, num_pieces: usize, piece_length: u32, last_piece_len: u32) -> Self {
        debug_assert!(
            piece_length.is_multiple_of(BLOCK_SIZE) || num_pieces == 0,
            "piece_length={piece_length} no es múltiplo de BLOCK_SIZE={BLOCK_SIZE}"
        );

        let block_state: Vec<Vec<BlockState>> = (0..num_pieces)
            .map(|pi| {
                let pl = if pi + 1 == num_pieces {
                    last_piece_len
                } else {
                    piece_length
                };
                let nb = pl.div_ceil(BLOCK_SIZE) as usize;
                vec![BlockState::Pending; nb]
            })
            .collect();

        let block_hashes = block_state.iter().map(|bs| vec![None; bs.len()]).collect();

        Self {
            fi,
            piece_length,
            last_piece_len,
            num_pieces,
            block_state,
            block_hashes,
            piece_done: vec![false; num_pieces],
            piece_verifying: vec![false; num_pieces],
            availability: vec![0u32; num_pieces],
            priority: Priority::Normal,
        }
    }

    /// Devuelve la prioridad actual del archivo.
    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Establece la prioridad de descarga del archivo completo.
    pub fn set_priority(&mut self, priority: Priority) {
        self.priority = priority;
    }
}

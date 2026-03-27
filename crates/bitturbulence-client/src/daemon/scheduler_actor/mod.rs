//! Actor que centraliza todos los accesos a los [`BlockScheduler`]s.
//!
//! ## Motivación
//!
//! Con el modelo anterior (`Vec<Mutex<BlockScheduler>>`), cada drainer adquiría
//! hasta N×11 locks por iteración de su bucle (N = número de archivos). Con
//! varios peers simultáneos, los schedulers se convertían en el punto de
//! contención central del pipeline de descarga.
//!
//! ## Diseño
//!
//! Un único task de Tokio posee todos los `BlockScheduler`s sin ningún lock.
//! Drainers y contexto se comunican con él mediante [`Msg`] y canales oneshot.
//! Toda la lógica de `pick_block` (11 passes) es síncrona dentro del actor.
//!
//! ## Garantías
//!
//! - Serialización total: no hay carreras de datos entre drainers.
//! - Sin inanición: el canal tiene capacidad 128; el actor procesa cada mensaje
//!   en tiempo O(N) donde N es el número de archivos.

mod pick;

use tokio::sync::{mpsc, oneshot};

use bitturbulence_pieces::BlockScheduler;

use crate::daemon::stream::PeerAvail;
use pick::pick_block_sync;

// ── Mensajes ──────────────────────────────────────────────────────────────────

enum Msg {
    // ── Drainer ──────────────────────────────────────────────────────────────
    /// Selecciona el siguiente bloque a descargar para el peer indicado.
    /// `endgame=true` habilita los passes 10-11 (bloques `InFlight`).
    PickBlock {
        peer_avail: Vec<PeerAvail>,
        endgame: bool,
        reply: oneshot::Sender<Option<bitturbulence_pieces::BlockTask>>,
    },
    /// Número total de bloques `Pending` en todos los archivos.
    TotalPending { reply: oneshot::Sender<u32> },
    /// Marca el bloque como completo y almacena su hash SHA-256.
    /// Devuelve `true` si es el primer stream en completar todos los bloques
    /// de la pieza (señal para iniciar la verificación Merkle).
    BlockDone {
        fi: usize,
        pi: u32,
        bi: u32,
        hash: [u8; 32],
        reply: oneshot::Sender<bool>,
    },
    /// Decrementa el contador in-flight de un bloque (fallo o stream muerto).
    BlockFailed { fi: usize, pi: u32, bi: u32 },
    /// Registra el bitfield de disponibilidad de un peer al conectar.
    AddPeerBitfield { fi: usize, bits: Vec<bool> },
    /// Elimina la contribución de un peer que se desconecta.
    RemovePeerBitfield { fi: usize, bits: Vec<bool> },
    /// El peer anuncia que ha descargado la pieza `pi`.
    AddPeerHave { fi: usize, pi: usize },

    // ── Context (verify_and_complete / is_complete) ───────────────────────────
    /// Devuelve los hashes SHA-256 de todos los bloques de la pieza `pi`.
    PieceBlockHashes {
        fi: usize,
        pi: u32,
        reply: oneshot::Sender<Option<Vec<[u8; 32]>>>,
    },
    /// Marca la pieza como verificada y descargada completamente.
    MarkPieceVerified { fi: usize, pi: u32 },
    /// La verificación Merkle ha fallado: resetea todos los bloques a `Pending`.
    MarkPieceHashFailed { fi: usize, pi: u32 },
    /// Devuelve `true` si todos los archivos del BitFlow están completos.
    IsComplete { reply: oneshot::Sender<bool> },
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Handle clonable para comunicarse con el [`SchedulerActor`].
///
/// Reemplaza a `Vec<Mutex<BlockScheduler>>` en [`FlowCtx`]. Todos los métodos
/// son `async` y devuelven inmediatamente en cuanto el actor los procesa.
#[derive(Clone)]
pub(crate) struct SchedulerHandle {
    tx: mpsc::Sender<Msg>,
}

impl SchedulerHandle {
    /// Lanza el actor en un nuevo task de Tokio y devuelve el handle.
    pub(crate) fn spawn(schedulers: Vec<BlockScheduler>) -> Self {
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(run_actor(schedulers, rx));
        Self { tx }
    }

    // ── Drainer ──────────────────────────────────────────────────────────────

    /// Selecciona el bloque más prioritario disponible para el peer.
    ///
    /// `endgame`: cuando es `true`, incluye bloques `InFlight` (passes 10-11)
    /// para redundancia en modo endgame o inestabilidad.
    pub(crate) async fn pick_block(
        &self,
        peer_avail: &[PeerAvail],
        endgame: bool,
    ) -> Option<bitturbulence_pieces::BlockTask> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(Msg::PickBlock {
                peer_avail: peer_avail.to_vec(),
                endgame,
                reply: tx,
            })
            .await;
        rx.await.unwrap_or(None)
    }

    /// Suma de bloques `Pending` en todos los archivos.
    pub(crate) async fn total_pending(&self) -> u32 {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Msg::TotalPending { reply: tx }).await;
        rx.await.unwrap_or(0)
    }

    /// Marca el bloque como completado con su hash SHA-256.
    /// Devuelve `true` si todos los bloques de la pieza están `Done`.
    pub(crate) async fn block_done(&self, fi: usize, pi: u32, bi: u32, hash: [u8; 32]) -> bool {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(Msg::BlockDone {
                fi,
                pi,
                bi,
                hash,
                reply: tx,
            })
            .await;
        rx.await.unwrap_or(false)
    }

    /// Decrementa el contador in-flight del bloque (fallo o stream muerto).
    pub(crate) async fn block_failed(&self, fi: usize, pi: u32, bi: u32) {
        let _ = self.tx.send(Msg::BlockFailed { fi, pi, bi }).await;
    }

    /// Registra el bitfield de disponibilidad de un peer al conectar.
    pub(crate) async fn add_peer_bitfield(&self, fi: usize, bits: Vec<bool>) {
        let _ = self.tx.send(Msg::AddPeerBitfield { fi, bits }).await;
    }

    /// Elimina la contribución de un peer que se desconecta.
    pub(crate) async fn remove_peer_bitfield(&self, fi: usize, bits: Vec<bool>) {
        let _ = self.tx.send(Msg::RemovePeerBitfield { fi, bits }).await;
    }

    /// Registra que el peer ha descargado la pieza `pi` del archivo `fi`.
    pub(crate) async fn add_peer_have(&self, fi: usize, pi: usize) {
        let _ = self.tx.send(Msg::AddPeerHave { fi, pi }).await;
    }

    // ── Context ───────────────────────────────────────────────────────────────

    /// Devuelve los hashes SHA-256 de todos los bloques de la pieza `pi`.
    pub(crate) async fn piece_block_hashes(&self, fi: usize, pi: u32) -> Option<Vec<[u8; 32]>> {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .tx
            .send(Msg::PieceBlockHashes { fi, pi, reply: tx })
            .await;
        rx.await.unwrap_or(None)
    }

    /// Marca la pieza como verificada y descargada completamente.
    pub(crate) async fn mark_piece_verified(&self, fi: usize, pi: u32) {
        let _ = self.tx.send(Msg::MarkPieceVerified { fi, pi }).await;
    }

    /// La verificación Merkle ha fallado: resetea todos los bloques a `Pending`.
    pub(crate) async fn mark_piece_hash_failed(&self, fi: usize, pi: u32) {
        let _ = self.tx.send(Msg::MarkPieceHashFailed { fi, pi }).await;
    }

    /// Devuelve `true` si todos los archivos del BitFlow están completos.
    pub(crate) async fn is_complete(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Msg::IsComplete { reply: tx }).await;
        rx.await.unwrap_or(false)
    }
}

// ── Actor ─────────────────────────────────────────────────────────────────────

async fn run_actor(mut schedulers: Vec<BlockScheduler>, mut rx: mpsc::Receiver<Msg>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::PickBlock {
                peer_avail,
                endgame,
                reply,
            } => {
                let task = pick_block_sync(&mut schedulers, &peer_avail, endgame);
                let _ = reply.send(task);
            }
            Msg::TotalPending { reply } => {
                let n: u32 = schedulers.iter().map(|s| s.pending_blocks()).sum();
                let _ = reply.send(n);
            }
            Msg::BlockDone {
                fi,
                pi,
                bi,
                hash,
                reply,
            } => {
                let ready = schedulers
                    .get_mut(fi)
                    .map(|s| s.mark_block_done(pi, bi, hash))
                    .unwrap_or(false);
                let _ = reply.send(ready);
            }
            Msg::BlockFailed { fi, pi, bi } => {
                if let Some(s) = schedulers.get_mut(fi) {
                    s.mark_block_failed(pi, bi);
                }
            }
            Msg::AddPeerBitfield { fi, bits } => {
                if let Some(s) = schedulers.get_mut(fi) {
                    s.add_peer_bitfield(&bits);
                }
            }
            Msg::RemovePeerBitfield { fi, bits } => {
                if let Some(s) = schedulers.get_mut(fi) {
                    s.remove_peer_bitfield(&bits);
                }
            }
            Msg::AddPeerHave { fi, pi } => {
                if let Some(s) = schedulers.get_mut(fi) {
                    s.add_peer_have(pi);
                }
            }
            Msg::PieceBlockHashes { fi, pi, reply } => {
                let hashes = schedulers.get(fi).and_then(|s| s.piece_block_hashes(pi));
                let _ = reply.send(hashes);
            }
            Msg::MarkPieceVerified { fi, pi } => {
                if let Some(s) = schedulers.get_mut(fi) {
                    s.mark_piece_verified(pi);
                }
            }
            Msg::MarkPieceHashFailed { fi, pi } => {
                if let Some(s) = schedulers.get_mut(fi) {
                    s.mark_piece_hash_failed(pi);
                }
            }
            Msg::IsComplete { reply } => {
                let ok = schedulers.iter().all(|s| s.is_complete());
                let _ = reply.send(ok);
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bitturbulence_pieces::BlockScheduler;
    use bitturbulence_protocol::BLOCK_SIZE;

    /// Un scheduler de 1 archivo, N piezas, cada pieza = 1 bloque (BLOCK_SIZE).
    fn make_handle(num_pieces: usize) -> SchedulerHandle {
        let sched = BlockScheduler::new(0, num_pieces, BLOCK_SIZE, BLOCK_SIZE);
        SchedulerHandle::spawn(vec![sched])
    }

    #[tokio::test]
    async fn is_complete_with_no_schedulers() {
        let handle = SchedulerHandle::spawn(vec![]);
        assert!(
            handle.is_complete().await,
            "sin schedulers = completo vacío"
        );
    }

    #[tokio::test]
    async fn is_complete_after_all_pieces_marked() {
        let handle = make_handle(2);
        assert!(!handle.is_complete().await);
        handle.mark_piece_verified(0, 0).await;
        assert!(!handle.is_complete().await, "falta la pieza 1");
        handle.mark_piece_verified(0, 1).await;
        assert!(handle.is_complete().await, "todas las piezas verificadas");
    }

    #[tokio::test]
    async fn block_done_signals_piece_ready_for_single_block_piece() {
        let handle = make_handle(1);
        // 1 pieza × 1 bloque → block_done con bi=0 devuelve true
        let ready = handle.block_done(0, 0, 0, [0xABu8; 32]).await;
        assert!(
            ready,
            "pieza de un solo bloque debe estar lista inmediatamente"
        );
    }

    #[tokio::test]
    async fn block_done_oob_fi_does_not_panic() {
        let handle = make_handle(1);
        // fi=99 está fuera de rango — no debe entrar en pánico
        let ready = handle.block_done(99, 0, 0, [0u8; 32]).await;
        assert!(!ready, "fi inválido devuelve false");
    }

    #[tokio::test]
    async fn block_failed_oob_fi_does_not_panic() {
        let handle = make_handle(1);
        handle.block_failed(99, 0, 0).await;
        handle.add_peer_bitfield(99, vec![true]).await;
        handle.remove_peer_bitfield(99, vec![true]).await;
        handle.add_peer_have(99, 0).await;
        // ninguno debe entrar en pánico
        assert!(!handle.is_complete().await);
    }

    #[tokio::test]
    async fn pick_block_returns_none_for_have_none_peer() {
        let handle = make_handle(2);
        let avail = vec![PeerAvail::HaveNone];
        let task = handle.pick_block(&avail, false).await;
        assert!(task.is_none(), "peer sin piezas → ningún bloque disponible");
    }

    #[tokio::test]
    async fn pick_block_returns_task_for_have_all_peer() {
        let handle = make_handle(2);
        let avail = vec![PeerAvail::HaveAll];
        let task = handle.pick_block(&avail, false).await;
        assert!(
            task.is_some(),
            "peer con todas las piezas → hay tarea disponible"
        );
    }

    #[tokio::test]
    async fn total_pending_decreases_on_verify() {
        let handle = make_handle(3);
        assert_eq!(handle.total_pending().await, 3);
        handle.mark_piece_verified(0, 0).await;
        assert_eq!(handle.total_pending().await, 2);
        handle.mark_piece_verified(0, 1).await;
        handle.mark_piece_verified(0, 2).await;
        assert_eq!(handle.total_pending().await, 0);
    }
}

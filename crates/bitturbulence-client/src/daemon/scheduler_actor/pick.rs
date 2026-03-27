//! Lógica de selección de bloque: aplica los 11 passes de prioridad × rareza.
//!
//! Completamente síncrona — sin locks ni async — porque el actor posee los
//! schedulers en exclusiva y la llama directamente.

use bitturbulence_pieces::BlockScheduler;
use bitturbulence_protocol::Priority;

use crate::daemon::stream::PeerAvail;

// ── Constantes de selección inter-archivo ─────────────────────────────────────

/// Umbral de "baja disponibilidad": el archivo lo tiene como máximo este número
/// de peers. Los archivos raros se priorizan sobre los comunes de igual prioridad.
const LOW_AVAIL: u32 = 1;

/// Passes de prioridad alta × rareza aplicados antes de los archivos normales.
///
/// Cada entrada es `(prioridad_mínima_del_archivo, máx_disponibilidad_permitida)`.
/// `None` en el segundo campo deshabilita el filtro de rareza.
pub(super) const PRIORITY_PASSES: &[(Priority, Option<u32>)] = &[
    (Priority::Maximum, Some(LOW_AVAIL)),  // 1. Máxima + rareza
    (Priority::VeryHigh, Some(LOW_AVAIL)), // 2. Muy alta + rareza
    (Priority::Maximum, None),             // 3. Máxima
    (Priority::Higher, Some(LOW_AVAIL)),   // 4. Más alta + rareza
    (Priority::VeryHigh, None),            // 5. Muy alta
    (Priority::High, Some(LOW_AVAIL)),     // 6. Alta + rareza
    (Priority::Higher, None),              // 7. Más alta
    (Priority::High, None),                // 8. Alta
];

/// Selecciona el bloque más prioritario disponible para el peer dado.
///
/// Aplica los 11 passes del protocolo de selección inter-archivo de forma
/// completamente síncrona, sin locks, ya que el actor posee los schedulers.
///
/// - Passes 1–8 : bloques `Pending` de archivos con prioridad alta × rareza.
/// - Pass 9     : bloques `Pending` de archivos con prioridad Normal o inferior.
/// - Passes 10–11: bloques `InFlight` (redundancia); solo si `endgame=true`.
pub(super) fn pick_block_sync(
    schedulers: &mut [BlockScheduler],
    peer_avail: &[PeerAvail],
    endgame: bool,
) -> Option<bitturbulence_pieces::BlockTask> {
    // Passes 1–8: Pending × prioridad alta × rareza.
    for &(prio, max_avail) in PRIORITY_PASSES {
        for (fi, avail) in peer_avail.iter().enumerate() {
            let Some(sched) = schedulers.get_mut(fi) else {
                continue;
            };
            if sched.priority() != prio {
                continue;
            }
            if max_avail.is_some_and(|m| sched.min_availability() > m) {
                continue;
            }
            let bits = avail.as_bitfield(sched.num_pieces());
            if bits.iter().all(|&h| !h) {
                continue;
            }
            if let Some(task) = sched.schedule_pending(&bits) {
                return Some(task);
            }
        }
    }

    // Pass 9: Pending de archivos con prioridad Normal o inferior.
    for (fi, avail) in peer_avail.iter().enumerate() {
        let Some(sched) = schedulers.get_mut(fi) else {
            continue;
        };
        if sched.priority() >= Priority::High {
            continue;
        }
        let bits = avail.as_bitfield(sched.num_pieces());
        if bits.iter().all(|&h| !h) {
            continue;
        }
        if let Some(task) = sched.schedule_pending(&bits) {
            return Some(task);
        }
    }

    // Passes 10–11: InFlight (redundancia/endgame).
    if endgame {
        for (fi, avail) in peer_avail.iter().enumerate() {
            let Some(sched) = schedulers.get_mut(fi) else {
                continue;
            };
            let bits = avail.as_bitfield(sched.num_pieces());
            if bits.iter().all(|&h| !h) {
                continue;
            }
            if let Some(task) = sched.schedule(&bits) {
                return Some(task);
            }
        }
    }

    None
}

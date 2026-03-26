use std::path::Path;
use std::sync::{Arc, atomic::{AtomicU64, AtomicUsize, Ordering}};

use anyhow::{Context, Result};
use tokio::sync::Mutex;
use tracing::{info, warn};

use bitturbulence_pieces::{BlockScheduler, TorrentStore, piece_root_from_block_hashes};
use bitturbulence_protocol::Metainfo;

use super::scheduler_actor::SchedulerHandle;

/// Estado compartido de un BitFlow activo.
pub struct FlowCtx {
    pub meta:  Metainfo,
    pub store: TorrentStore,
    /// Actor que gestiona los BlockSchedulers de todos los archivos sin locks.
    pub sched: SchedulerHandle,
    /// have[fi][pi] = pieza verificada y completa (para anunciar a peers).
    pub have:  Mutex<Vec<Vec<bool>>>,
    /// Bytes descargados y verificados.
    pub downloaded: AtomicU64,
    /// Peers conectados actualmente.
    pub peer_count: AtomicUsize,
}

impl FlowCtx {
    pub async fn new(meta: Metainfo, save_path: &Path, seeding: bool) -> Result<Arc<Self>> {
        let store = TorrentStore::open(save_path, &meta).await
            .context("opening torrent store")?;

        let mut raw_schedulers = Vec::with_capacity(meta.files.len());
        let mut have_init      = Vec::with_capacity(meta.files.len());

        for fi in 0..meta.files.len() {
            let file    = store.file(fi);
            let num     = file.num_pieces() as usize;
            let pl      = meta.files[fi].piece_length();
            let last_pl = meta.files[fi].last_piece_length();

            let mut sched = BlockScheduler::new(fi, num, pl, last_pl);

            let file_have = if seeding {
                for pi in 0..num { sched.mark_piece_verified(pi as u32); }
                vec![true; num]
            } else {
                vec![false; num]
            };

            raw_schedulers.push(sched);
            have_init.push(file_have);
        }

        Ok(Arc::new(Self {
            sched: SchedulerHandle::spawn(raw_schedulers),
            meta,
            store,
            have: Mutex::new(have_init),
            downloaded: AtomicU64::new(0),
            peer_count: AtomicUsize::new(0),
        }))
    }

    /// Bitfield de piezas que tenemos completas para el archivo `fi`.
    pub async fn our_bitfield(&self, fi: usize) -> Vec<bool> {
        self.have.lock().await[fi].clone()
    }

    /// Verifica la raíz Merkle de la pieza usando los hashes de bloque
    /// almacenados en el actor (sin releer el disco) y la marca completa.
    ///
    /// Devuelve `true` si la verificación fue correcta.
    /// En caso de fallo resetea todos los bloques de la pieza a `Pending`.
    pub async fn verify_and_complete(&self, fi: usize, pi: u32) -> bool {
        let block_hashes = match self.sched.piece_block_hashes(fi, pi).await {
            Some(h) => h,
            None => {
                warn!(fi, pi, "piece_block_hashes unavailable");
                self.sched.mark_piece_hash_failed(fi, pi).await;
                return false;
            }
        };
        let expected    = &self.meta.files[fi].piece_hashes[pi as usize];
        let computed    = piece_root_from_block_hashes(&block_hashes);
        let piece_bytes = self.meta.files[fi].piece_len(pi) as u64;
        if &computed == expected {
            self.have.lock().await[fi][pi as usize] = true;
            self.sched.mark_piece_verified(fi, pi).await;
            self.downloaded.fetch_add(piece_bytes, Ordering::Relaxed);
            info!(fi, pi, bytes = piece_bytes, "piece merkle root ok");
            true
        } else {
            warn!(fi, pi, "merkle root mismatch — resetting piece to Pending");
            self.sched.mark_piece_hash_failed(fi, pi).await;
            false
        }
    }

    pub async fn is_complete(&self) -> bool {
        self.sched.is_complete().await
    }
}

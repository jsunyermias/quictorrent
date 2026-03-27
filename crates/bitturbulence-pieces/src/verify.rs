use sha2::{Digest, Sha256};

// ── Funciones primitivas ──────────────────────────────────────────────────────

/// SHA-256 de datos arbitrarios.
/// Úsalo para info_hash o hashes de propósito general, NO para piezas.
pub fn hash_block(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Alias de [`hash_block`] para compatibilidad con código existente.
#[inline]
pub fn hash_piece(data: &[u8]) -> [u8; 32] {
    hash_block(data)
}

// ── Árbol Merkle ──────────────────────────────────────────────────────────────

/// Nodo padre en el árbol Merkle: SHA-256(left || right).
pub fn merkle_parent(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// Raíz Merkle de una lista de hashes.
///
/// Si la longitud no es potencia de 2, se rellena con hashes cero (`[0u8; 32]`)
/// hasta la siguiente potencia de 2, igual que en BitTorrent v2.
///
/// - Lista vacía → `[0u8; 32]`
/// - Lista de 1 → el único elemento (sin hash extra)
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    if leaves.len() == 1 {
        return leaves[0];
    }
    let n = leaves.len().next_power_of_two();
    let mut layer: Vec<[u8; 32]> = Vec::with_capacity(n);
    layer.extend_from_slice(leaves);
    layer.resize(n, [0u8; 32]);

    while layer.len() > 1 {
        layer = layer
            .chunks(2)
            .map(|pair| merkle_parent(&pair[0], &pair[1]))
            .collect();
    }
    layer[0]
}

// ── Nivel de bloque ───────────────────────────────────────────────────────────

/// Verifica un bloque individual comparando su SHA-256 con el hash esperado.
pub fn verify_block(data: &[u8], expected_hash: &[u8; 32]) -> bool {
    &hash_block(data) == expected_hash
}

// ── Nivel de pieza ────────────────────────────────────────────────────────────

/// Raíz Merkle de una pieza: SHA-256 de cada bloque (16 KiB) → raíz Merkle.
///
/// El último bloque puede ser más corto que `BLOCK_SIZE`; se hashea tal cual.
pub fn piece_root(piece_data: &[u8]) -> [u8; 32] {
    use bitturbulence_protocol::BLOCK_SIZE;
    let block_hashes: Vec<[u8; 32]> = piece_data
        .chunks(BLOCK_SIZE as usize)
        .map(hash_block)
        .collect();
    merkle_root(&block_hashes)
}

/// Raíz Merkle de una pieza a partir de los hashes de sus bloques individuales
/// (cuando no tenemos los datos en memoria, solo los hashes).
pub fn piece_root_from_block_hashes(block_hashes: &[[u8; 32]]) -> [u8; 32] {
    merkle_root(block_hashes)
}

/// Verifica una pieza completa calculando su raíz Merkle de bloques y
/// comparándola con la raíz esperada del metainfo.
pub fn verify_piece(piece_data: &[u8], expected_root: &[u8; 32]) -> bool {
    &piece_root(piece_data) == expected_root
}

// ── Nivel de archivo ──────────────────────────────────────────────────────────

/// Raíz Merkle de un archivo: raíz Merkle de las raíces de pieza.
pub fn file_root(piece_roots: &[[u8; 32]]) -> [u8; 32] {
    merkle_root(piece_roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_correct() {
        let data: Vec<u8> = (0..16 * 1024).map(|i| (i % 251) as u8).collect();
        let root = piece_root(&data);
        assert!(verify_piece(&data, &root));
    }

    #[test]
    fn verify_wrong() {
        let data: Vec<u8> = (0..16 * 1024).map(|i| (i % 251) as u8).collect();
        assert!(!verify_piece(&data, &[0u8; 32]));
    }

    #[test]
    fn hash_is_32_bytes() {
        let h = hash_block(b"test");
        assert_eq!(h.len(), 32);
    }

    #[test]
    fn merkle_root_single_leaf() {
        let leaf = [1u8; 32];
        assert_eq!(merkle_root(&[leaf]), leaf);
    }

    #[test]
    fn merkle_root_two_leaves() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let expected = merkle_parent(&a, &b);
        assert_eq!(merkle_root(&[a, b]), expected);
    }

    #[test]
    fn merkle_root_pads_to_pow2() {
        // 3 leaves → padded to 4 with zero hash
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        let zero = [0u8; 32];

        let expected = merkle_parent(&merkle_parent(&a, &b), &merkle_parent(&c, &zero));
        assert_eq!(merkle_root(&[a, b, c]), expected);
    }

    #[test]
    fn piece_root_deterministic() {
        let data: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
        assert_eq!(piece_root(&data), piece_root(&data));
        // Diferente de SHA-256 plano
        assert_ne!(piece_root(&data), hash_block(&data));
    }

    #[test]
    fn verify_block_correct() {
        let data = b"hello block";
        let h = hash_block(data);
        assert!(verify_block(data, &h));
    }

    #[test]
    fn file_root_uses_merkle() {
        let p0 = [0xAAu8; 32];
        let p1 = [0xBBu8; 32];
        assert_eq!(file_root(&[p0, p1]), merkle_parent(&p0, &p1));
    }
}

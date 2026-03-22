/// Estado choke/interest de un lado de la conexión.
/// Cada sesión tiene dos instancias: local (nosotros) y remote (el peer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChokeState {
    /// Si estamos choked por el otro lado (no podemos pedir bloques).
    pub choked: bool,
    /// Si estamos interested en el otro lado (queremos datos suyos).
    pub interested: bool,
}

impl Default for ChokeState {
    /// BEP-3: estado inicial siempre es choked y not-interested.
    fn default() -> Self {
        Self { choked: true, interested: false }
    }
}

impl ChokeState {
    /// Podemos descargar si no estamos choked y estamos interested.
    pub fn can_download(&self) -> bool {
        !self.choked && self.interested
    }
}

/// Estado completo de una sesión peer-to-peer.
#[derive(Debug, Clone)]
pub struct SessionState {
    /// Lo que el peer remoto nos impone a nosotros.
    pub remote_to_local: ChokeState,
    /// Lo que nosotros imponemos al peer remoto.
    pub local_to_remote: ChokeState,
    /// Piezas que tiene el peer (bitfield).
    pub peer_pieces: Vec<bool>,
    /// Número de piezas totales del torrent.
    pub num_pieces: usize,
    /// Requests en vuelo que hemos enviado al peer.
    pub pending_requests: Vec<PendingRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub piece_index: u32,
    pub begin: u32,
    pub length: u32,
}

impl SessionState {
    pub fn new(num_pieces: usize) -> Self {
        Self {
            remote_to_local: ChokeState::default(),
            local_to_remote: ChokeState::default(),
            peer_pieces: vec![false; num_pieces],
            num_pieces,
            pending_requests: Vec::new(),
        }
    }

    /// El peer nos ha hecho choke — cancelamos requests pendientes.
    pub fn apply_choke(&mut self) {
        self.remote_to_local.choked = true;
        self.pending_requests.clear();
    }

    pub fn apply_unchoke(&mut self) {
        self.remote_to_local.choked = false;
    }

    pub fn apply_interested(&mut self) {
        self.local_to_remote.interested = true;
    }

    pub fn apply_not_interested(&mut self) {
        self.local_to_remote.interested = false;
    }

    pub fn apply_have(&mut self, piece_index: u32) {
        let idx = piece_index as usize;
        if idx < self.peer_pieces.len() {
            self.peer_pieces[idx] = true;
        }
    }

    pub fn apply_bitfield(&mut self, bitfield: &[u8]) {
        for (byte_idx, byte) in bitfield.iter().enumerate() {
            for bit in 0..8 {
                let piece_idx = byte_idx * 8 + bit;
                if piece_idx >= self.num_pieces {
                    break;
                }
                self.peer_pieces[piece_idx] = (byte >> (7 - bit)) & 1 == 1;
            }
        }
    }

    pub fn peer_has_piece(&self, index: usize) -> bool {
        self.peer_pieces.get(index).copied().unwrap_or(false)
    }

    pub fn can_request(&self) -> bool {
        self.remote_to_local.can_download()
    }

    pub fn add_pending_request(&mut self, req: PendingRequest) {
        self.pending_requests.push(req);
    }

    pub fn remove_pending_request(&mut self, piece_index: u32, begin: u32) {
        self.pending_requests.retain(|r| {
            !(r.piece_index == piece_index && r.begin == begin)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_choked() {
        let s = SessionState::new(100);
        assert!(!s.can_request());
        assert!(s.remote_to_local.choked);
        assert!(!s.remote_to_local.interested);
    }

    #[test]
    fn unchoke_allows_requests() {
        let mut s = SessionState::new(100);
        s.remote_to_local.interested = true;
        s.apply_unchoke();
        assert!(s.can_request());
    }

    #[test]
    fn choke_clears_pending() {
        let mut s = SessionState::new(100);
        s.pending_requests.push(PendingRequest { piece_index: 0, begin: 0, length: 16384 });
        s.apply_choke();
        assert!(s.pending_requests.is_empty());
    }

    #[test]
    fn bitfield_parsed_correctly() {
        let mut s = SessionState::new(16);
        // 0b11000000 0b00000001 => piezas 0,1,15
        s.apply_bitfield(&[0b1100_0000, 0b0000_0001]);
        assert!(s.peer_has_piece(0));
        assert!(s.peer_has_piece(1));
        assert!(!s.peer_has_piece(2));
        assert!(s.peer_has_piece(15));
    }

    #[test]
    fn have_updates_pieces() {
        let mut s = SessionState::new(100);
        assert!(!s.peer_has_piece(42));
        s.apply_have(42);
        assert!(s.peer_has_piece(42));
    }
}

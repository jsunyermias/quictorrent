# quictorrent
BitTorrent v2 over QUIC, written in Rust.

## Crates
- `qt-protocol` — BitTorrent v2 protocol (messages, metainfo, info hash)
- `qt-transport` — QUIC transport layer (quinn)
- `qt-peer` — Peer management and sessions
- `qt-pieces` — Piece picking, verification and storage
- `qt-tracker` — Tracker client and server
- `qt-dht` — DHT (Kademlia)
- `qt-client` — CLI binary

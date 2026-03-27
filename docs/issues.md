# Issues abiertos — bitturbulence

Ordenados por prioridad de implementación. Actualizado: 2026-03-27. Última revisión completa: 2026-03-27 (commit d6c705f).

| # | Título | Prioridad |
|---|--------|-----------|
| [#48](https://github.com/jsunyermias/bitturbulence/issues/48) | `submarine flow set-priority`: IPC + CLI para cambiar prioridad de archivo en caliente | 🟠 Media-alta |
| [#49](https://github.com/jsunyermias/bitturbulence/issues/49) | `flow create --priority <fi>:<level>`: predefinir prioridades al generar el .bitflow | 🟠 Media-alta |
| [#50](https://github.com/jsunyermias/bitturbulence/issues/50) | Coordinador multi-archivo por prioridad: drainer elige archivos según Priority antes de piezas | 🟠 Media-alta |
| [#54](https://github.com/jsunyermias/bitturbulence/issues/54) | Token bucket por archivo en FlowCtx: tasa de descarga configurable (bytes/s) | 🟠 Media |
| [#55](https://github.com/jsunyermias/bitturbulence/issues/55) | IPC/CLI `submarine flow throttle <flow-id> <fi> <bytes/s>`: ajustar throttle en caliente | 🟠 Media |
| [#56](https://github.com/jsunyermias/bitturbulence/issues/56) | Aplicar throttle en el drainer: respetar token bucket entre Requests por archivo | 🟠 Media |
| [#41](https://github.com/jsunyermias/bitturbulence/issues/41) | Límite de conexiones simultáneas de peers + evicción LRU | 🟠 Media-alta |
| [#43](https://github.com/jsunyermias/bitturbulence/issues/43) | Validar longitud de bloque en mensajes Piece recibidos | 🟠 Media |
| [#42](https://github.com/jsunyermias/bitturbulence/issues/42) | Blacklisting de peers por mismatches de hash Merkle reiterados | 🟠 Media |
| [#40](https://github.com/jsunyermias/bitturbulence/issues/40) | Backoff exponencial en reconexión de peers (1s → 2s → 4s … 60s) | 🟠 Media |
| [#45](https://github.com/jsunyermias/bitturbulence/issues/45) | Métricas de throughput: velocidad (B/s) por peer y global | 🟠 Media |
| [#39](https://github.com/jsunyermias/bitturbulence/issues/39) | PriorityHint: reordenar descarga del scheduler al recibir cambio de prioridad | 🟠 Media |
| [#46](https://github.com/jsunyermias/bitturbulence/issues/46) | Progreso por archivo en respuesta IPC Status (% completado por file_index) | 🟡 Media-baja |
| [#23](https://github.com/jsunyermias/bitturbulence/issues/23) | `submarine status`: progreso en tiempo real (velocidad, ETA, peers activos) | 🟡 Media-baja |
| [#30](https://github.com/jsunyermias/bitturbulence/issues/30) | Velocímetro: estadísticas de velocidad en tiempo real | 🟡 Media-baja |
| [#47](https://github.com/jsunyermias/bitturbulence/issues/47) | Hot-reload de config del daemon sin reinicio (SIGHUP o IPC reload) | 🟡 Media-baja |
| [#57](https://github.com/jsunyermias/bitturbulence/issues/57) | STUN client: resolver IP pública y puerto externo (RFC 5389) | 🟡 Media-baja |
| [#58](https://github.com/jsunyermias/bitturbulence/issues/58) | Hole punching coordinado via tracker: intercambio de candidatos antes de conectar | 🟡 Media-baja |
| [#59](https://github.com/jsunyermias/bitturbulence/issues/59) | TURN relay fallback cuando hole punching falla (RFC 5766) | 🟡 Media-baja |
| [#60](https://github.com/jsunyermias/bitturbulence/issues/60) | Keepalive adaptativo para móviles: reducir KEEPALIVE_INTERVAL en background/batería | 🟡 Media-baja |
| [#61](https://github.com/jsunyermias/bitturbulence/issues/61) | QUIC connection migration: mantener sesión al cambiar de red (WiFi ↔ 4G) | 🟡 Media-baja |
| [#51](https://github.com/jsunyermias/bitturbulence/issues/51) | Índice global de bloques por hash: content-addressable store para deduplicación | 🟡 Media-baja |
| [#52](https://github.com/jsunyermias/bitturbulence/issues/52) | Deduplicación al añadir flow: detectar y marcar piezas ya presentes en disco | 🟡 Media-baja |
| [#53](https://github.com/jsunyermias/bitturbulence/issues/53) | Reference counting de bloques compartidos: no borrar datos al eliminar un flow | 🟡 Media-baja |
| [#18](https://github.com/jsunyermias/bitturbulence/issues/18) | NAT traversal (épico, ver #57–#59 para subissues) | 🟡 Media-baja |
| [#13](https://github.com/jsunyermias/bitturbulence/issues/13) | Cliente tracker HTTP/UDP estándar (compat. BitTorrent) | 🟡 Baja |
| [#33](https://github.com/jsunyermias/bitturbulence/issues/33) | Streaming: prioridad automática de primeras piezas para reproducción progresiva | 🔵 Baja |
| [#8](https://github.com/jsunyermias/bitturbulence/issues/8) | Choke/unchoke: gestión de slots de subida | 🔵 Baja |
| [#62](https://github.com/jsunyermias/bitturbulence/issues/62) | Trait `BitTurbulencePlugin` con hooks: on_piece_verified, on_peer_connected, on_flow_complete | 🔵 Baja |
| [#63](https://github.com/jsunyermias/bitturbulence/issues/63) | Carga dinámica de plugins (.so / cdylib) vía libloading | 🔵 Baja |
| [#64](https://github.com/jsunyermias/bitturbulence/issues/64) | Plugin de ejemplo: logger de estadísticas (progreso, velocidad, peers) a CSV/JSON | 🔵 Baja |
| [#20](https://github.com/jsunyermias/bitturbulence/issues/20) | Limitación de velocidad: throttling global de subida/bajada | 🔵 Baja |
| [#7](https://github.com/jsunyermias/bitturbulence/issues/7) | Pool de conexiones de peers con límite global | 🔵 Baja |
| [#22](https://github.com/jsunyermias/bitturbulence/issues/22) | Vortex links: compartir flows sin `.bitflow` (`vortex://`) | 🔵 Baja |
| [#37](https://github.com/jsunyermias/bitturbulence/issues/37) | Web UI: panel de control con progreso en tiempo real via WebSocket | 🔵 Baja |
| [#25](https://github.com/jsunyermias/bitturbulence/issues/25) | Renombrar repositorio de quictorrent a bitturbulence | 🔵 Admin |
| [#34](https://github.com/jsunyermias/bitturbulence/issues/34) | Soporte Windows: compilación y CI en Windows | 🔵 Baja |
| [#35](https://github.com/jsunyermias/bitturbulence/issues/35) | Packaging: .deb, .rpm, Flatpak y Homebrew | 🔵 Baja |

## Cerrados recientemente

| # | Título | Commit |
|---|--------|--------|
| [#38](https://github.com/jsunyermias/bitturbulence/issues/38) | Filler: procesar mensajes Cancel para evitar servir bloques ya cancelados | 159307d |
| [#44](https://github.com/jsunyermias/bitturbulence/issues/44) | Logging estructurado: tracing spans con peer_id/addr/info_hash por conexión | 8555d15 |
| [#27](https://github.com/jsunyermias/bitturbulence/issues/27) | CI pipeline: clippy --deny warnings + cargo test + rustfmt | f4f3458 |
| [#32](https://github.com/jsunyermias/bitturbulence/issues/32) | Verificación Merkle incremental via HashRequest/HashResponse | 1b5d3ee |
| [#26](https://github.com/jsunyermias/bitturbulence/issues/26) | Multiplexación adaptativa: scale-down de data streams | 72f5a65 |
| [#28](https://github.com/jsunyermias/bitturbulence/issues/28) | Daemon en background + IPC Unix socket | 4f3f8c8 |
| [#21](https://github.com/jsunyermias/bitturbulence/issues/21) | `submarine flow create`: herramienta nativa Rust para generar `.bitflow` | 9fd9553 |
| — | Batería de tests: scheduler_actor, context, have_persist (154 tests total) | 779619d |
| [#36](https://github.com/jsunyermias/bitturbulence/issues/36) | Seeder automático: pasar a modo Seeding al completar la descarga | f4c90ee |
| [#29](https://github.com/jsunyermias/bitturbulence/issues/29) | Integrar DHT con descubrimiento de peers en el daemon | c9a8746 |
| [#31](https://github.com/jsunyermias/bitturbulence/issues/31) | Propagación de HavePiece a todos los peers conectados | e3de67d |
| [#19](https://github.com/jsunyermias/bitturbulence/issues/19) | Reanudación de descargas: persistencia del have bitfield | 6723f3b |

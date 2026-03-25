use std::path::PathBuf;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "submarine",
    about = "Submarine — cliente BitTurbulence",
    version,
    propagate_version = true,
)]
pub struct Cli {
    /// Fichero de configuración (por defecto ~/.submarine/config.json).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Gestión de torrents.
    Torrent {
        #[command(subcommand)]
        action: TorrentAction,
    },
    /// Muestra el estado de todas las descargas.
    Status,
    /// Arranca el servidor (tracker + DHT + QUIC endpoint).
    Serve,
}

#[derive(Subcommand)]
pub enum TorrentAction {
    /// Añade un torrent a la cola.
    Add {
        /// Fichero .bitflow (BitFlow metainfo JSON).
        path: PathBuf,
        /// Directorio de descarga (sobreescribe el valor por defecto).
        #[arg(short, long)]
        save: Option<PathBuf>,
        /// Prioridad inicial (0=minimum .. 8=maximum, por defecto 4=normal).
        #[arg(short, long, default_value = "4")]
        priority: u8,
    },
    /// Inicia o reanuda la descarga de un torrent.
    Start {
        /// ID del torrent (primeros 8 chars del info_hash).
        id: String,
    },
    /// Pausa la descarga de un torrent.
    Pause {
        id: String,
    },
    /// Elimina un torrent de la lista.
    Stop {
        id: String,
    },
    /// Muestra los peers conectados a un torrent.
    Peers {
        id: String,
    },
}

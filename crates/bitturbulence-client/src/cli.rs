use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "submarine",
    about = "Submarine — cliente BitTurbulence",
    version,
    propagate_version = true
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
    /// Gestión de BitFlows.
    Flow {
        #[command(subcommand)]
        action: FlowAction,
    },
    /// Muestra el estado de todas las descargas.
    Status,
    /// Arranca el servidor (tracker + DHT + QUIC endpoint).
    Serve {
        /// Ejecutar el daemon en segundo plano.
        #[arg(short = 'd', long)]
        background: bool,
        /// Detener el daemon en ejecución.
        #[arg(long, conflicts_with = "background")]
        stop: bool,
    },
}

#[derive(Subcommand)]
pub enum FlowAction {
    /// Añade un BitFlow a la cola.
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
    /// Inicia o reanuda la descarga de un BitFlow.
    Start {
        /// ID del BitFlow (primeros 8 chars del info_hash).
        id: String,
    },
    /// Pausa la descarga de un BitFlow.
    Pause { id: String },
    /// Elimina un BitFlow de la lista.
    Stop { id: String },
    /// Muestra los peers conectados a un BitFlow.
    Peers { id: String },
    /// Crea un fichero .bitflow a partir de un archivo o directorio.
    Create {
        /// Archivo o directorio a empaquetar.
        path: PathBuf,
        /// Nombre del torrent (por defecto: nombre del archivo/directorio).
        #[arg(short, long)]
        name: Option<String>,
        /// URL de tracker (repetible: --tracker url1 --tracker url2).
        #[arg(short = 't', long = "tracker")]
        trackers: Vec<String>,
        /// Comentario opcional.
        #[arg(short, long)]
        comment: Option<String>,
        /// Prioridad (0=minimum .. 8=maximum, por defecto 4=normal).
        #[arg(short, long, default_value = "4")]
        priority: u8,
        /// Directorio de salida del .bitflow (por defecto: directorio padre del path).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

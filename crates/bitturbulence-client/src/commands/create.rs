use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use bitturbulence_pieces::piece_root;
use bitturbulence_protocol::{piece_length_for_size, FileEntry, Metainfo, Priority};

use super::format::format_size;

/// Crea un fichero .bitflow a partir de un archivo o directorio.
pub fn cmd_create(
    path: &Path,
    name: Option<String>,
    trackers: Vec<String>,
    comment: Option<String>,
    priority: Priority,
    output: Option<PathBuf>,
) -> Result<()> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("resolving path {:?}", path))?;

    let default_name = abs
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let name = name.unwrap_or(default_name);

    let files: Vec<FileEntry> = if abs.is_file() {
        let data = std::fs::read(&abs).with_context(|| format!("reading {:?}", abs))?;
        let fname = abs
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        vec![build_file_entry(vec![fname], &data, priority)]
    } else if abs.is_dir() {
        let mut raw: Vec<(Vec<String>, Vec<u8>)> = vec![];
        collect_dir_files(&abs, &abs, &mut raw)?;
        raw.sort_by(|(a, _), (b, _)| a.cmp(b));
        raw.into_iter()
            .map(|(p, d)| build_file_entry(p, &d, priority))
            .collect()
    } else {
        return Err(anyhow!("not a file or directory: {:?}", abs));
    };

    if files.is_empty() {
        return Err(anyhow!("no files found in {:?}", abs));
    }

    // Cada tracker CLI se convierte en su propio tier (BEP 12).
    // Así un fallo de un tracker no bloquea los demás.
    let tracker_tiers: Vec<Vec<String>> = trackers.iter().map(|t| vec![t.clone()]).collect();
    let mut meta = Metainfo {
        name: name.clone(),
        info_hash: [0u8; 32],
        files,
        trackers,
        tracker_tiers,
        comment,
    };
    meta.info_hash = meta.compute_info_hash();

    let hash_hex = hex::encode(meta.info_hash);
    let out_dir = output.unwrap_or_else(|| abs.parent().unwrap_or(Path::new(".")).to_path_buf());
    std::fs::create_dir_all(&out_dir)?;
    let out_path = out_dir.join(format!("{}.bitflow", &hash_hex[..8]));

    let json = serde_json::to_string_pretty(&meta)?;
    std::fs::write(&out_path, &json).with_context(|| format!("writing {:?}", out_path))?;

    println!("created:  {}", out_path.display());
    println!("name:     {}", meta.name);
    println!("hash:     {}", hash_hex);
    println!("files:    {}", meta.files.len());
    println!("size:     {}", format_size(meta.total_size()));
    println!("pieces:   {}", meta.total_pieces());

    Ok(())
}

fn build_file_entry(path: Vec<String>, data: &[u8], priority: Priority) -> FileEntry {
    let size = data.len() as u64;
    let pl = piece_length_for_size(size) as usize;
    let piece_hashes = if data.is_empty() {
        vec![]
    } else {
        data.chunks(pl).map(piece_root).collect()
    };
    FileEntry {
        path,
        size,
        piece_hashes,
        priority,
    }
}

fn collect_dir_files(root: &Path, dir: &Path, out: &mut Vec<(Vec<String>, Vec<u8>)>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading directory {:?}", dir))?
        .collect::<std::io::Result<_>>()
        .context("enumerating directory entries")?;
    entries.sort_by_key(|e| e.file_name());

    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect_dir_files(root, &p, out)?;
        } else if p.is_file() {
            let components: Vec<String> = p
                .strip_prefix(root)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let data = std::fs::read(&p).with_context(|| format!("reading {:?}", p))?;
            out.push((components, data));
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_single_file_produces_valid_bitflow() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.bin");
        std::fs::write(
            &file,
            b"hello world this is test data for the bitflow creator",
        )
        .unwrap();

        let out_dir = dir.path().join("out");
        cmd_create(
            &file,
            None,
            vec![],
            None,
            Priority::Normal,
            Some(out_dir.clone()),
        )
        .unwrap();

        let entries: Vec<_> = std::fs::read_dir(&out_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("bitflow"))
            .collect();
        assert_eq!(entries.len(), 1, "debe haber exactamente 1 .bitflow");

        let json = std::fs::read(&entries[0].path()).unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        assert_eq!(meta.name, "hello.bin");
        assert_eq!(meta.files.len(), 1);
        assert_eq!(meta.files[0].path, vec!["hello.bin"]);
        assert_ne!(meta.info_hash, [0u8; 32], "info_hash debe estar calculado");
    }

    #[test]
    fn create_info_hash_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, &vec![0x42u8; 64 * 1024]).unwrap();

        let out1 = dir.path().join("out1");
        let out2 = dir.path().join("out2");
        cmd_create(&file, None, vec![], None, Priority::Normal, Some(out1.clone())).unwrap();
        cmd_create(&file, None, vec![], None, Priority::Normal, Some(out2.clone())).unwrap();

        let meta1: Metainfo = serde_json::from_slice(
            &std::fs::read(
                std::fs::read_dir(&out1).unwrap().next().unwrap().unwrap().path(),
            )
            .unwrap(),
        )
        .unwrap();
        let meta2: Metainfo = serde_json::from_slice(
            &std::fs::read(
                std::fs::read_dir(&out2).unwrap().next().unwrap().unwrap().path(),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(meta1.info_hash, meta2.info_hash, "mismo contenido → mismo info_hash");
    }

    #[test]
    fn create_directory_includes_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("mydir");
        std::fs::create_dir_all(src.join("subdir")).unwrap();
        std::fs::write(src.join("a.txt"), b"archivo a").unwrap();
        std::fs::write(src.join("b.txt"), b"archivo b").unwrap();
        std::fs::write(src.join("subdir").join("c.txt"), b"archivo c").unwrap();

        let out = dir.path().join("out");
        cmd_create(&src, None, vec![], None, Priority::Normal, Some(out.clone())).unwrap();

        let json = std::fs::read(
            std::fs::read_dir(&out).unwrap().next().unwrap().unwrap().path(),
        )
        .unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        assert_eq!(meta.name, "mydir");
        assert_eq!(meta.files.len(), 3);
        let paths: Vec<&Vec<String>> = meta.files.iter().map(|f| &f.path).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "archivos deben estar en orden determinista");
    }

    #[test]
    fn create_with_trackers_and_comment() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.bin");
        std::fs::write(&file, b"data").unwrap();

        let out = dir.path().join("out");
        cmd_create(
            &file,
            Some("mi torrent".into()),
            vec!["https://tracker.example.com/announce".into()],
            Some("comentario de prueba".into()),
            Priority::High,
            Some(out.clone()),
        )
        .unwrap();

        let json = std::fs::read(
            std::fs::read_dir(&out).unwrap().next().unwrap().unwrap().path(),
        )
        .unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        assert_eq!(meta.name, "mi torrent");
        assert_eq!(meta.trackers, vec!["https://tracker.example.com/announce"]);
        assert_eq!(meta.comment, Some("comentario de prueba".into()));
    }

    #[test]
    fn create_piece_hashes_match_verify() {
        use bitturbulence_pieces::verify_piece;
        use bitturbulence_protocol::piece_length_for_size;

        let data = vec![0xABu8; 32 * 1024]; // 32 KB
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, &data).unwrap();

        let out = dir.path().join("out");
        cmd_create(&file, None, vec![], None, Priority::Normal, Some(out.clone())).unwrap();

        let json = std::fs::read(
            std::fs::read_dir(&out).unwrap().next().unwrap().unwrap().path(),
        )
        .unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        let pl = piece_length_for_size(data.len() as u64) as usize;
        for (pi, chunk) in data.chunks(pl).enumerate() {
            assert!(
                verify_piece(chunk, &meta.files[0].piece_hashes[pi]),
                "pieza {pi} debe verificarse con los hashes generados"
            );
        }
    }
}

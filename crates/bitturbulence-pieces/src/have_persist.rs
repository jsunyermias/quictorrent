//! Persistencia del bitfield `have[][]` entre sesiones.
//!
//! Formato: un fichero JSON `{save_path}/.have.json`
//! Contenido: `[[bool]]` — have[fi][pi]

use anyhow::{Context, Result};
use std::path::Path;

/// Guarda el bitfield have en `{save_path}/.have.json`.
pub fn save_have(save_path: &Path, have: &[Vec<bool>]) -> Result<()> {
    let path = save_path.join(".have.json");
    let json = serde_json::to_string(have).context("serializing have")?;
    std::fs::write(&path, json).context("writing .have.json")?;
    Ok(())
}

/// Carga el bitfield have desde `{save_path}/.have.json`.
/// Si el fichero no existe, devuelve `None`.
pub fn load_have(save_path: &Path) -> Result<Option<Vec<Vec<bool>>>> {
    let path = save_path.join(".have.json");
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path).context("reading .have.json")?;
    let have: Vec<Vec<bool>> = serde_json::from_str(&json).context("parsing .have.json")?;
    Ok(Some(have))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let have = vec![vec![true, false, true], vec![false, false]];
        save_have(dir.path(), &have).unwrap();
        let loaded = load_have(dir.path()).unwrap().unwrap();
        assert_eq!(have, loaded);
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_have(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn partial_have_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let have = vec![vec![true, false, false, true]];
        save_have(dir.path(), &have).unwrap();
        let loaded = load_have(dir.path()).unwrap().unwrap();
        assert_eq!(loaded[0], vec![true, false, false, true]);
    }

    #[test]
    fn load_corrupted_json_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".have.json"), b"not valid json {{{{").unwrap();
        assert!(
            load_have(dir.path()).is_err(),
            "JSON malformado debe devolver error"
        );
    }

    #[test]
    fn save_and_load_empty_bitfield() {
        let dir = tempfile::tempdir().unwrap();
        let have: Vec<Vec<bool>> = vec![];
        save_have(dir.path(), &have).unwrap();
        let loaded = load_have(dir.path()).unwrap().unwrap();
        assert!(loaded.is_empty());
    }
}

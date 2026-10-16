//! dono: C9
//! Persistência do Config em TOML: caminho, padrões, migração e gravação atômica.
//! A estrutura `Config` vive em `types::config` (F2). Stubs do F2: assinaturas.

use std::path::{Path, PathBuf};

use crate::types::Config;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("não implementado")]
    NotImplemented,
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Parse(String),
}

/// `~/Library/Application Support/Relay/config.toml` no Mac; `%APPDATA%\Relay\config.toml` no Windows.
pub fn config_path() -> PathBuf {
    PathBuf::from("config.toml")
}

/// Carrega o arquivo; cria com os padrões se não existe; renomeia para `.bak` se inválido.
pub fn load_or_default(_path: &Path) -> Result<Config, ConfigError> {
    Ok(Config::default())
}

/// Grava de forma atômica (temporário + rename).
pub fn save(_path: &Path, _config: &Config) -> Result<(), ConfigError> {
    Err(ConfigError::NotImplemented)
}

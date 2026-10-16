//! Log em arquivo (`%LOCALAPPDATA%\Relay\logs\agent.log`), com rotação simples por tamanho: ao passar de `MAX_BYTES`,
//! `agent.log` vira `agent.log.1` (o anterior é descartado). Mesmo desenho de `apps/windows-service/src/logging.rs`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

/// Tamanho a partir do qual o arquivo é rotacionado.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Diretório de logs: `%LOCALAPPDATA%\Relay\logs` (fallback: temp).
pub fn log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(
        || std::env::temp_dir().join("Relay").join("logs"),
        |b| PathBuf::from(b).join("Relay").join("logs"),
    )
}

/// Arquivo de log com rotação por tamanho.
pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    file: File,
    size: u64,
}

impl RotatingFile {
    pub fn open(path: &Path, max_bytes: u64) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            max_bytes,
            file,
            size,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        let mut backup = self.path.clone().into_os_string();
        backup.push(".1");
        // No Windows `rename` falha se o destino existe.
        let _ = fs::remove_file(&backup);
        fs::rename(&self.path, &backup)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.size = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            // Falha ao rotacionar não pode derrubar o log: segue escrevendo no arquivo atual.
            let _ = self.rotate();
        }
        let n = self.file.write(buf)?;
        self.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Handle compartilhável entre threads para o `tracing-subscriber`.
#[derive(Clone)]
pub struct SharedLog(Arc<Mutex<RotatingFile>>);

pub struct LogWriter(Arc<Mutex<RotatingFile>>);

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

impl<'a> MakeWriter<'a> for SharedLog {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> LogWriter {
        LogWriter(Arc::clone(&self.0))
    }
}

/// Instala o subscriber global gravando em `<log_dir>/agent.log`. Best-effort: sem permissão de
/// escrita o app segue sem log (o erro volta ao chamador, que o ignora).
pub fn init() -> io::Result<()> {
    let file = RotatingFile::open(&log_dir().join("agent.log"), MAX_BYTES)?;
    let writer = SharedLog(Arc::new(Mutex::new(file)));
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotaciona_ao_passar_do_limite() {
        let dir = std::env::temp_dir().join(format!("relay-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("logs").join("agent.log");
        let mut f = RotatingFile::open(&path, 10).unwrap();
        f.write_all(b"123456").unwrap();
        f.write_all(b"789012").unwrap(); // 12 > 10: rotaciona antes
        f.flush().unwrap();
        assert_eq!(
            fs::read(dir.join("logs").join("agent.log.1")).unwrap(),
            b"123456"
        );
        assert_eq!(fs::read(&path).unwrap(), b"789012");
        let _ = fs::remove_dir_all(&dir);
    }
}

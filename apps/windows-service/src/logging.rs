//! Log em arquivo do relay-service (`%ProgramData%\Relay\logs\service.log`), com rotação simples
//! por tamanho: ao passar de `MAX_BYTES`, `service.log` vira `service.log.1` (o anterior é
//! descartado). Sem processo de limpeza e sem dependência extra além do `tracing-subscriber`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

/// Tamanho a partir do qual o arquivo é rotacionado.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Diretório de logs: `%ProgramData%\Relay\logs` (fallback `C:\ProgramData`).
pub fn log_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    base.join("Relay").join("logs")
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

/// Instala o subscriber global gravando em `<log_dir>/<file_name>`. Best-effort: sem permissão de
/// escrita (ex.: helper em sessão de usuário) o serviço segue sem log, e o erro volta ao chamador.
pub fn init(file_name: &str) -> io::Result<()> {
    let file = RotatingFile::open(&log_dir().join(file_name), MAX_BYTES)?;
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

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("relay-log-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn rotaciona_ao_passar_do_limite() {
        let dir = tmp("rot");
        let path = dir.join("logs").join("service.log");
        let mut f = RotatingFile::open(&path, 10).unwrap();
        f.write_all(b"123456").unwrap();
        f.write_all(b"789012").unwrap(); // 12 > 10: rotaciona antes
        f.flush().unwrap();
        assert_eq!(
            fs::read(dir.join("logs").join("service.log.1")).unwrap(),
            b"123456"
        );
        assert_eq!(fs::read(&path).unwrap(), b"789012");
        // Segunda rotação substitui o backup anterior.
        f.write_all(b"abcdef").unwrap();
        assert_eq!(
            fs::read(dir.join("logs").join("service.log.1")).unwrap(),
            b"789012"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reabre_em_modo_append_e_conta_tamanho_existente() {
        let dir = tmp("app");
        let path = dir.join("service.log");
        RotatingFile::open(&path, 100)
            .unwrap()
            .write_all(b"antes\n")
            .unwrap();
        let mut f = RotatingFile::open(&path, 100).unwrap();
        f.write_all(b"depois\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"antes\ndepois\n");
        let _ = fs::remove_dir_all(&dir);
    }
}

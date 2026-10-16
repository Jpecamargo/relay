//! dono: M2
//! Detecção de Secure Input (IsSecureEventInputEnabled a cada 500 ms).

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

fn system_secure_input() -> bool {
    // SAFETY: chamada sem ponteiros.
    unsafe { IsSecureEventInputEnabled() }
}

/// Detector de mudança: guarda o último valor (começa em `false`) e só devolve `Some` quando
/// o valor lido difere dele.
#[derive(Debug, Default)]
pub struct ChangeDetector {
    last: bool,
}

impl ChangeDetector {
    pub fn update(&mut self, now: bool) -> Option<bool> {
        if now == self.last {
            None
        } else {
            self.last = now;
            Some(now)
        }
    }
}

type Source = Arc<dyn Fn() -> bool + Send + Sync>;

/// Consulta periódica; chama o callback só quando o valor muda. Se o Secure Input já estiver
/// ativo ao iniciar, o callback dispara uma vez com `true`.
pub struct SecureInputWatcher {
    source: Source,
    interval: Duration,
    stop_tx: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for SecureInputWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureInputWatcher")
            .field("running", &self.thread.is_some())
            .finish()
    }
}

impl Default for SecureInputWatcher {
    fn default() -> Self {
        Self::with_source(Arc::new(system_secure_input), POLL_INTERVAL)
    }
}

impl SecureInputWatcher {
    /// Fonte e intervalo injetáveis (testes).
    pub fn with_source(source: Source, interval: Duration) -> Self {
        Self {
            source,
            interval,
            stop_tx: None,
            thread: None,
        }
    }

    pub fn start(&mut self, on_change: Box<dyn Fn(bool) + Send + Sync>) {
        self.stop();
        let (tx, rx) = mpsc::channel::<()>();
        let source = Arc::clone(&self.source);
        let interval = self.interval;
        self.stop_tx = Some(tx);
        self.thread = Some(std::thread::spawn(move || {
            let mut det = ChangeDetector::default();
            loop {
                if let Some(v) = det.update(source()) {
                    on_change(v);
                }
                // O timeout é o intervalo; o envio (ou o drop) do canal acorda para parar.
                match rx.recv_timeout(interval) {
                    Err(RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            }
        }));
    }

    pub fn stop(&mut self) {
        self.stop_tx = None; // fecha o canal e acorda a thread
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SecureInputWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn detector_so_reporta_mudanca() {
        let mut d = ChangeDetector::default();
        assert_eq!(d.update(false), None);
        assert_eq!(d.update(true), Some(true));
        assert_eq!(d.update(true), None);
        assert_eq!(d.update(false), Some(false));
        assert_eq!(d.update(false), None);
    }

    #[test]
    fn watcher_chama_callback_so_quando_muda() {
        let flag = Arc::new(AtomicBool::new(false));
        let f = Arc::clone(&flag);
        let mut w = SecureInputWatcher::with_source(
            Arc::new(move || f.load(Ordering::SeqCst)),
            Duration::from_millis(5),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = Arc::clone(&seen);
        w.start(Box::new(move |v| s.lock().unwrap().push(v)));

        let wait = || std::thread::sleep(Duration::from_millis(60));
        wait(); // várias leituras com false: nada
        flag.store(true, Ordering::SeqCst);
        wait(); // várias leituras com true: uma chamada
        flag.store(false, Ordering::SeqCst);
        wait();
        w.stop();
        assert_eq!(*seen.lock().unwrap(), vec![true, false]);
    }

    #[test]
    fn stop_e_start_repetidos_nao_travam() {
        let mut w = SecureInputWatcher::default();
        w.start(Box::new(|_| {}));
        w.start(Box::new(|_| {}));
        w.stop();
        w.stop();
    }
}

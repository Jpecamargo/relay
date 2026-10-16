//! dono: F2
//! Traits de plataforma. Implementadas pelos backends (`relay-platform-macos`,
//! `relay-platform-windows`) e por mocks de teste; consumidas pelo engine, pelo AppCore (Mac) e
//! pelo AgentCore (Windows). Regras comuns: todas são `Send`; nenhuma faz chamada `async`; os
//! métodos do caminho de evento (`InputCapture`, `InputInjector`, `CursorControl`) não alocam
//! nem bloqueiam. Callbacks podem ser chamados de threads do sistema.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::thread::Thread;
use std::time::Duration;

use crate::types::{
    AudioDevice, CapturedEvent, ClipboardContent, CommandResult, DisplayInfo, FileEntry,
    InputEvent, RemoteCommand,
};

/// Callback sem argumentos chamado de uma thread do sistema (mudança de telas, de clipboard).
pub type ChangeCallback = Box<dyn Fn() + Send + Sync>;

/// Amostras por canal de um quadro de áudio: 5 ms a 48 kHz (D13).
pub const AUDIO_FRAME_SAMPLES: usize = 240;
pub const AUDIO_CHANNELS: usize = 2;
/// Quadro de áudio de 5 ms, PCM s16 estéreo intercalado, 48 kHz.
pub type AudioFrame = [i16; AUDIO_FRAME_SAMPLES * AUDIO_CHANNELS];

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// Sem permissão de Monitoramento de Entrada / Acessibilidade.
    #[error("permissão negada")]
    PermissionDenied,
    #[error("captura já iniciada")]
    AlreadyRunning,
    #[error("não implementado")]
    NotImplemented,
    #[error("{0}")]
    Platform(String),
}

#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    #[error("não implementado")]
    NotImplemented,
    #[error("{0}")]
    Platform(String),
}

/// Erro genérico dos demais backends (cursor, telas, clipboard, áudio, executor).
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("não implementado")]
    NotImplemented,
    #[error("não suportado nesta plataforma ou versão")]
    Unsupported,
    #[error("{0}")]
    Platform(String),
}

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("{0}")]
    Backend(String),
}

/// Acordador do laço que consome a captura: o callback do sistema chama `wake` e a thread do laço
/// dorme em `park` em vez de sondar. Sem alocação nem lock (só `Thread::unpark`); o "token" do
/// `unpark` vale mesmo se a thread ainda não estacionou, então não há acordada perdida.
#[derive(Clone, Default)]
pub struct LoopWaker(Arc<OnceLock<Thread>>);

impl LoopWaker {
    /// Chamado pela thread do laço antes do primeiro `park`: é ela que será acordada.
    pub fn bind_current(&self) {
        let _ = self.0.set(std::thread::current());
    }

    pub fn wake(&self) {
        if let Some(t) = self.0.get() {
            t.unpark();
        }
    }

    /// Dorme até `timeout` ou até alguém chamar `wake` (acordadas espúrias são possíveis).
    pub fn park(timeout: Duration) {
        std::thread::park_timeout(timeout);
    }
}

impl std::fmt::Debug for LoopWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoopWaker")
    }
}

/// Captura global de input (CGEventTap no Mac, hooks LL no Windows).
pub trait InputCapture: Send {
    /// Inicia a captura em thread própria. Cada evento vira um `CapturedEvent` empurrado em
    /// `sink` (sem lock nem alocação). Eventos reconhecidos como atalho continuam sendo
    /// entregues ao `sink` mesmo com `swallow` ligado.
    fn start(&mut self, sink: rtrb::Producer<CapturedEvent>) -> Result<(), CaptureError>;
    /// Registra quem acordar a cada evento empurrado em `sink` (chame antes de `start`). Sem
    /// isso o laço só vê o evento na próxima volta do seu relógio.
    fn set_waker(&mut self, _waker: LoopWaker) {}
    fn stop(&mut self);
    /// Liga/desliga o "engolir": com `true`, o evento não chega ao sistema local.
    /// Usa estado atômico interno; pode ser chamado de qualquer thread.
    fn set_swallow(&self, swallow: bool);
    fn is_swallowing(&self) -> bool;
}

/// Delay e taxa de repetição de tecla do sistema de destino (D12).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyRepeat {
    pub delay_ms: u32,
    pub rate_hz: f32,
}

impl Default for KeyRepeat {
    fn default() -> Self {
        Self {
            delay_ms: 500,
            rate_hz: 30.0,
        }
    }
}

/// Injeção de input no sistema local (SendInput no Windows, CGEventPost no Mac).
pub trait InputInjector: Send {
    /// Injeta um evento. Movimento relativo é limitado à união das telas.
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError>;
    /// Digita texto Unicode (`RemoteCommand::TypeText`).
    fn type_text(&mut self, text: &str) -> Result<(), InjectError>;
    /// Delay e taxa de repetição de tecla deste sistema.
    fn key_repeat(&self) -> KeyRepeat {
        KeyRepeat::default()
    }
}

/// Controle do cursor local.
pub trait CursorControl: Send {
    fn hide(&mut self) -> Result<(), PlatformError>;
    fn show(&mut self) -> Result<(), PlatformError>;
    /// `false` desassocia o mouse físico do cursor (Mac); `true` restaura.
    fn associate(&mut self, associated: bool) -> Result<(), PlatformError>;
    /// Move o cursor para `(x, y)` no espaço de coordenadas das telas (ver `DisplayInfo`).
    fn warp(&mut self, x: i32, y: i32) -> Result<(), PlatformError>;
    /// Posição atual do cursor no mesmo espaço.
    fn position(&self) -> Result<(i32, i32), PlatformError>;
}

/// Telas da máquina local.
pub trait DisplaySource: Send {
    /// Telas ativas, com `machine` preenchido pelo backend.
    fn list(&self) -> Result<Vec<DisplayInfo>, PlatformError>;
    /// Registra o callback de mudança de configuração de telas (um só; substitui o anterior).
    fn set_change_callback(&mut self, callback: ChangeCallback);
}

/// Clipboard do sistema local.
pub trait ClipboardBackend: Send {
    /// Lê o conteúdo atual (`None` se vazio ou sem formato suportado). Para `FileList`,
    /// guarda os caminhos para `local_paths` (`FileEntry.id` = índice).
    fn read(&mut self) -> Result<Option<ClipboardContent>, PlatformError>;
    /// Escreve conteúdo vindo do remoto, marcando-o (RelayRemote / com.joaocamargo.relay.remote)
    /// para que o callback de mudança o ignore.
    fn write_remote(&mut self, content: ClipboardContent) -> Result<(), PlatformError>;
    /// Registra o callback de mudança local (não dispara para escritas marcadas).
    fn set_change_callback(&mut self, callback: ChangeCallback);
    /// Caminhos locais da última `FileList` lida; o índice é o `FileEntry.id`.
    fn local_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    /// Registra o provedor de arquivos remotos: chamado (bloqueando) quando o destino cola uma
    /// `FileList` escrita por `write_remote` (renderização atrasada no Windows) e devolve os
    /// caminhos locais já baixados. Backends sem renderização atrasada ignoram.
    fn set_file_provider(&mut self, _provider: FileProvider) {}
}

/// Provedor bloqueante de arquivos remotos (ver `ClipboardBackend::set_file_provider`).
pub type FileProvider = Box<dyn Fn(&[FileEntry]) -> Result<Vec<PathBuf>, String> + Send + Sync>;

/// Captura de áudio (loopback do PC no Windows, microfone no Mac) em quadros de 5 ms.
pub trait AudioCapture: Send {
    /// Inicia a captura. Cada quadro de exatamente 240 amostras por canal vai para `sink`.
    fn start(&mut self, sink: rtrb::Producer<AudioFrame>) -> Result<(), PlatformError>;
    fn stop(&mut self);
    /// Troca o dispositivo em execução (`None` = padrão do sistema).
    fn set_device(&mut self, device_id: Option<&str>) -> Result<(), PlatformError>;
}

/// Reprodução de áudio a partir de um ring sem lock de f32 intercalado, 48 kHz estéreo.
pub trait AudioPlayback: Send {
    fn start(&mut self, source: rtrb::Consumer<f32>) -> Result<(), PlatformError>;
    fn stop(&mut self);
    /// Ganho do stream, 0.0..=1.0.
    fn set_volume(&mut self, volume: f32);
    /// Troca o dispositivo em execução (`None` = padrão do sistema).
    fn set_device(&mut self, device_id: Option<&str>) -> Result<(), PlatformError>;
    fn list_devices(&self) -> Result<Vec<AudioDevice>, PlatformError>;
}

/// Executa um `RemoteCommand` no Windows (agente) e produz o `CommandResult` com o mesmo `id`.
pub trait CommandExecutor: Send {
    fn execute(&mut self, id: u32, command: &RemoteCommand) -> CommandResult;
}

/// Armazenamento de segredos (Keychain / Credential Manager), service `com.joaocamargo.relay`.
/// Contas: `identity` (chave X25519 privada) e `peer:<device_id>` (`PairedPeer` serializado).
pub trait KeyStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, KeyStoreError>;
    fn set(&self, account: &str, secret: &[u8]) -> Result<(), KeyStoreError>;
    fn delete(&self, account: &str) -> Result<(), KeyStoreError>;
}

//! dono: W7
//! Registro do serviço RelayService no SCM e helper de sessão (usado por apps/windows-service).
//!
//! Arquitetura (D10): o serviço (SYSTEM) só supervisiona. Em cada sessão ativa ele lança
//! `relay-service.exe --session-helper` com o token SYSTEM duplicado e movido para a sessão do
//! console; o helper segue o desktop de input (UAC, tela de bloqueio) e injeta o que chega pelo
//! pipe `relay-inject`. A lógica sem Win32 (SDDL, backoff, decisão de relançar) é pura e testada
//! no Mac; o resto fica em `win`, sob `cfg(windows)`.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Nome do serviço no SCM (glossário).
pub const SERVICE_NAME: &str = "RelayService";
pub const SERVICE_DISPLAY_NAME: &str = "Relay Input Service";
pub const SERVICE_DESCRIPTION: &str =
    "Permite que o teclado e o mouse do Relay funcionem no UAC e na tela de bloqueio.";
/// Argumento que faz o executável atuar como helper de sessão.
pub const HELPER_ARG: &str = "--session-helper";
/// Valor devolvido por `WTSGetActiveConsoleSessionId` quando não há sessão de console.
pub const NO_SESSION: u32 = 0xFFFF_FFFF;

/// SDDL do pipe: acesso total ao SYSTEM e escrita só ao usuário interativo da sessão. Sem SID
/// (ninguém logado), só o SYSTEM. DACL protegida: não herda nada.
pub fn pipe_sddl(user_sid: Option<&str>) -> String {
    match user_sid {
        Some(sid) => format!("D:P(A;;GA;;;SY)(A;;0x100002;;;{sid})"),
        None => "D:P(A;;GA;;;SY)".to_string(),
    }
}

/// Linha de comando do helper.
pub fn helper_command_line(exe: &str) -> String {
    format!("\"{exe}\" {HELPER_ARG}")
}

/// Recuo exponencial para relançar o helper: 250 ms, 500 ms, ... até 10 s.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub const BASE: Duration = Duration::from_millis(250);
    pub const MAX: Duration = Duration::from_secs(10);

    pub fn delay_for(attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
        Self::BASE.saturating_mul(factor).min(Self::MAX)
    }

    /// Atraso antes da próxima tentativa; cada chamada avança o recuo.
    pub fn next_delay(&mut self) -> Duration {
        let d = Self::delay_for(self.attempt);
        self.attempt = self.attempt.saturating_add(1);
        d
    }

    /// Zera o recuo (o helper ficou de pé por tempo suficiente).
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Tempo de vida a partir do qual um helper conta como estável e zera o recuo.
pub const STABLE_AFTER: Duration = Duration::from_secs(10);

/// O supervisor deve (re)lançar o helper? `helper_session` é a sessão do helper vivo, se houver.
pub fn needs_relaunch(helper_session: Option<u32>, active_session: u32) -> bool {
    active_session != NO_SESSION && helper_session != Some(active_session)
}

/// Nomes de imagem aceitos para o agente instalado (binário do cargo e nome do produto).
pub const AGENT_IMAGE_NAMES: [&str; 2] = ["relay-windows-agent.exe", "relay agent.exe"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDenied {
    /// O diretório do próprio serviço não está sob Program Files: nada garante a integridade.
    InstallDirUntrusted,
    /// A imagem do cliente não está no diretório do produto.
    WrongDirectory,
    /// A imagem do cliente não é o agente.
    WrongImage,
}

fn lowered(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

/// Decide se o cliente do pipe é o agente instalado. Recebe caminhos já canônicos (o chamador
/// usa `fs::canonicalize`); a comparação ignora maiúsculas (NTFS) e é por componente.
pub fn verify_client_image(
    client_image: &Path,
    install_dir: &Path,
    trusted_roots: &[PathBuf],
) -> Result<(), ClientDenied> {
    let dir = lowered(install_dir);
    let under_root = trusted_roots.iter().any(|r| {
        let r = lowered(r);
        dir != r && dir.starts_with(&r)
    });
    if !under_root {
        return Err(ClientDenied::InstallDirUntrusted);
    }
    let image = lowered(client_image);
    if image.parent() != Some(dir.as_path()) {
        return Err(ClientDenied::WrongDirectory);
    }
    let name = image.file_name().map(|n| n.to_string_lossy().into_owned());
    match name {
        Some(n) if AGENT_IMAGE_NAMES.contains(&n.as_str()) => Ok(()),
        _ => Err(ClientDenied::WrongImage),
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::time::Instant;

    use windows::Win32::Foundation::{
        CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL, LocalFree, WAIT_TIMEOUT,
    };
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        DuplicateTokenEx, GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        SecurityImpersonation, SetTokenInformation, TOKEN_ALL_ACCESS, TOKEN_USER, TokenPrimary,
        TokenSessionId, TokenUser,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, ReadFile,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
        PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::RemoteDesktop::{
        ProcessIdToSessionId, WTSGetActiveConsoleSessionId, WTSQueryUserToken,
    };
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_ACCESS_FLAGS, GetThreadDesktop, GetUserObjectInformationW, HDESK,
        OpenInputDesktop, SetThreadDesktop, UOI_NAME,
    };
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateProcessAsUserW, GetCurrentProcess, GetCurrentProcessId,
        GetCurrentThreadId, OpenProcess, OpenProcessToken, PROCESS_INFORMATION, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, STARTUPINFOW,
        TerminateProcess, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};
    use windows_service::service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
        ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    use super::*;
    use crate::inject::WindowsInjector;
    use crate::pipe::{FrameAssembler, InjectFrame, PIPE_NAME};
    use relay_core::platform::InputInjector;

    pub type ServiceResult<T> = Result<T, String>;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn werr(what: &str, e: windows::core::Error) -> String {
        format!("{what}: {e}")
    }

    // ---- registro no SCM -------------------------------------------------------------------

    /// Cria o serviço `RelayService` (início automático, LocalSystem) e o inicia.
    pub fn install() -> ServiceResult<()> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .map_err(|e| format!("abrir o SCM: {e}"))?;
        let info = ServiceInfo {
            name: SERVICE_NAME.into(),
            display_name: SERVICE_DISPLAY_NAME.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe,
            launch_arguments: vec!["run".into()],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let service = manager
            .create_service(
                &info,
                ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|e| format!("criar o serviço: {e}"))?;
        // Descrição e início imediato são conveniências: falhar neles não desfaz a instalação.
        let _ = service.set_description(SERVICE_DESCRIPTION);
        if let Err(e) = service.start::<&OsStr>(&[]) {
            tracing::warn!("serviço instalado, mas não iniciou: {e}");
        }
        Ok(())
    }

    /// Para (se estiver rodando) e remove o serviço.
    pub fn uninstall() -> ServiceResult<()> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|e| format!("abrir o SCM: {e}"))?;
        let service = manager
            .open_service(
                SERVICE_NAME,
                ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|e| format!("abrir o serviço: {e}"))?;
        if let Ok(status) = service.query_status()
            && status.current_state != ServiceState::Stopped
        {
            let _ = service.stop();
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                match service.query_status() {
                    Ok(s) if s.current_state != ServiceState::Stopped => {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    _ => break,
                }
            }
        }
        service
            .delete()
            .map_err(|e| format!("remover o serviço: {e}"))
    }

    // ---- lançamento do helper na sessão ativa ----------------------------------------------

    /// Sessão do console, se houver.
    pub fn active_console_session() -> Option<u32> {
        // SAFETY: sem argumentos nem ponteiros.
        let id = unsafe { WTSGetActiveConsoleSessionId() };
        (id != NO_SESSION).then_some(id)
    }

    /// Processo do helper lançado pelo supervisor.
    pub struct HelperProcess {
        handle: HANDLE,
        pub session: u32,
        pub started: Instant,
    }

    impl HelperProcess {
        pub fn is_running(&self) -> bool {
            // SAFETY: `handle` é um handle de processo válido que este struct possui.
            unsafe { WaitForSingleObject(self.handle, 0) == WAIT_TIMEOUT }
        }

        pub fn terminate(&self) {
            // SAFETY: idem; falha (já encerrado) é ignorável.
            unsafe {
                let _ = TerminateProcess(self.handle, 0);
            }
        }
    }

    impl Drop for HelperProcess {
        fn drop(&mut self) {
            // SAFETY: fecha o handle que o struct possui, uma única vez.
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }

    /// Lança `relay-service.exe --session-helper` em `winsta0\default` da sessão `session`, com o
    /// token SYSTEM do serviço duplicado e `TokenSessionId` ajustado.
    pub fn launch_helper(session: u32) -> ServiceResult<HelperProcess> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let mut cmdline = wide(&helper_command_line(&exe.to_string_lossy()));
        let mut desktop = wide("winsta0\\default");
        // SAFETY: todas as chamadas recebem ponteiros para locais vivos; os handles abertos aqui
        // são fechados em todos os caminhos antes de retornar.
        unsafe {
            let mut own = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut own)
                .map_err(|e| werr("OpenProcessToken", e))?;
            let mut dup = HANDLE::default();
            let dup_res = DuplicateTokenEx(
                own,
                TOKEN_ALL_ACCESS,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut dup,
            );
            let _ = CloseHandle(own);
            dup_res.map_err(|e| werr("DuplicateTokenEx", e))?;

            let result = (|| {
                SetTokenInformation(
                    dup,
                    TokenSessionId,
                    std::ptr::from_ref(&session).cast(),
                    size_of::<u32>() as u32,
                )
                .map_err(|e| werr("SetTokenInformation(TokenSessionId)", e))?;
                let si = STARTUPINFOW {
                    cb: size_of::<STARTUPINFOW>() as u32,
                    lpDesktop: PWSTR(desktop.as_mut_ptr()),
                    ..Default::default()
                };
                let mut pi = PROCESS_INFORMATION::default();
                CreateProcessAsUserW(
                    Some(dup),
                    PCWSTR::null(),
                    Some(PWSTR(cmdline.as_mut_ptr())),
                    None,
                    None,
                    false,
                    CREATE_NO_WINDOW,
                    None,
                    PCWSTR::null(),
                    &si,
                    &mut pi,
                )
                .map_err(|e| werr("CreateProcessAsUserW", e))?;
                let _ = CloseHandle(pi.hThread);
                Ok(HelperProcess {
                    handle: pi.hProcess,
                    session,
                    started: Instant::now(),
                })
            })();
            let _ = CloseHandle(dup);
            result
        }
    }

    // ---- helper: servidor do pipe ----------------------------------------------------------

    /// SID (texto) do usuário logado na sessão deste processo, se houver.
    fn session_user_sid() -> Option<String> {
        // SAFETY: buffers locais dimensionados pela própria API; handle fechado ao fim.
        unsafe {
            let mut session = 0u32;
            ProcessIdToSessionId(GetCurrentProcessId(), &mut session).ok()?;
            let mut token = HANDLE::default();
            WTSQueryUserToken(session, &mut token).ok()?;
            let sid = (|| {
                let mut len = 0u32;
                let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
                if len == 0 {
                    return None;
                }
                // u64 garante o alinhamento exigido por TOKEN_USER.
                let mut buf = vec![0u64; (len as usize).div_ceil(8)];
                GetTokenInformation(
                    token,
                    TokenUser,
                    Some(buf.as_mut_ptr().cast()),
                    len,
                    &mut len,
                )
                .ok()?;
                let user = &*(buf.as_ptr().cast::<TOKEN_USER>());
                let mut s = PWSTR::null();
                ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
                let out = s.to_string().ok();
                let _ = LocalFree(Some(HLOCAL(s.0.cast())));
                out
            })();
            let _ = CloseHandle(token);
            sid
        }
    }

    fn name_of(desktop: HDESK) -> Option<String> {
        let mut buf = [0u16; 256];
        let mut needed = 0u32;
        // SAFETY: o buffer tem 512 bytes, informados em `nindex`/tamanho.
        unsafe {
            GetUserObjectInformationW(
                windows::Win32::Foundation::HANDLE(desktop.0),
                UOI_NAME,
                Some(buf.as_mut_ptr().cast()),
                (buf.len() * 2) as u32,
                Some(&mut needed),
            )
            .ok()?;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }

    /// Mantém a thread no desktop de input; troca (`SetThreadDesktop`) só quando ele muda.
    struct DesktopFollower {
        current: Option<HDESK>,
        current_name: Option<String>,
    }

    impl DesktopFollower {
        fn new() -> Self {
            // SAFETY: consulta o desktop atual desta thread.
            let initial = unsafe { GetThreadDesktop(GetCurrentThreadId()) }.ok();
            Self {
                current_name: initial.and_then(name_of),
                current: None, // o desktop inicial pertence ao processo: nunca é fechado aqui
            }
        }

        /// Alinha a thread ao desktop de input. Falhar (ex.: desktop Secure sem acesso) mantém o
        /// desktop anterior: a injeção segue, só não alcança aquele desktop.
        fn follow(&mut self) {
            // SAFETY: handles abertos aqui são fechados ou guardados em `current`.
            unsafe {
                let Ok(input) = OpenInputDesktop(
                    Default::default(),
                    false,
                    DESKTOP_ACCESS_FLAGS(0x1000_0000), // GENERIC_ALL
                ) else {
                    return;
                };
                let name = name_of(input);
                if name.is_some() && name == self.current_name {
                    let _ = CloseDesktop(input);
                    return;
                }
                if SetThreadDesktop(input).is_ok() {
                    if let Some(old) = self.current.replace(input) {
                        let _ = CloseDesktop(old);
                    }
                    self.current_name = name;
                } else {
                    let _ = CloseDesktop(input);
                }
            }
        }
    }

    fn create_pipe(first: bool, sddl: &str) -> ServiceResult<HANDLE> {
        let sddl_w = wide(sddl);
        // SAFETY: o descritor alocado pela API é liberado com LocalFree após CreateNamedPipeW.
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl_w.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .map_err(|e| werr("SDDL do pipe", e))?;
            let attrs = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd.0,
                bInheritHandle: false.into(),
            };
            let name = wide(PIPE_NAME);
            let mut open_mode = PIPE_ACCESS_INBOUND;
            if first {
                open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
            }
            let h = CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                2,
                0,
                64 * 1024,
                0,
                Some(&attrs),
            );
            let _ = LocalFree(Some(HLOCAL(sd.0)));
            if h.is_invalid() {
                return Err(werr(
                    "CreateNamedPipeW",
                    windows::core::Error::from_thread(),
                ));
            }
            Ok(h)
        }
    }

    /// Caminho canônico da imagem do processo cliente do pipe.
    fn client_image(pipe: HANDLE) -> Option<PathBuf> {
        // SAFETY: `pid` e o buffer vivem durante as chamadas; o handle é fechado ao fim.
        unsafe {
            let mut pid = 0u32;
            GetNamedPipeClientProcessId(pipe, &mut pid).ok()?;
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let r = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(h);
            r.ok()?;
            let raw = String::from_utf16_lossy(&buf[..len as usize]);
            std::fs::canonicalize(raw).ok()
        }
    }

    fn trusted_roots() -> Vec<PathBuf> {
        ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .filter_map(|v| std::fs::canonicalize(v).ok())
            .collect()
    }

    /// Só o agente instalado (imagem no diretório deste executável, sob Program Files) é aceito.
    fn client_is_agent(pipe: HANDLE, install_dir: Option<&Path>) -> bool {
        let Some(dir) = install_dir else {
            tracing::warn!("diretório de instalação desconhecido: cliente recusado");
            return false;
        };
        let Some(image) = client_image(pipe) else {
            tracing::warn!("imagem do cliente do pipe ilegível: recusado");
            return false;
        };
        match verify_client_image(&image, dir, &trusted_roots()) {
            Ok(()) => true,
            Err(why) => {
                tracing::warn!("cliente do pipe recusado ({why:?}): {}", image.display());
                false
            }
        }
    }

    /// Corpo do `--session-helper`: serve o pipe até o processo ser encerrado. Um cliente por vez.
    /// Sempre há uma instância a escutar (a próxima nasce antes de servir a atual), e a primeira
    /// é criada com FIRST_PIPE_INSTANCE: ninguém sequestra o nome. Cliente que não é o agente, quadro
    /// inválido ou queda derrubam só aquela conexão.
    pub fn run_helper() -> ServiceResult<()> {
        let sddl = pipe_sddl(session_user_sid().as_deref());
        let install_dir = std::env::current_exe()
            .ok()
            .and_then(|e| std::fs::canonicalize(e).ok())
            .and_then(|e| e.parent().map(Path::to_path_buf));
        let mut injector = WindowsInjector::default();
        let mut desktop = DesktopFollower::new();
        let mut listening = create_pipe(true, &sddl)?;
        loop {
            // SAFETY: `listening` é válido até ser fechado abaixo.
            let connected = unsafe {
                match ConnectNamedPipe(listening, None) {
                    Ok(()) => true,
                    Err(_) => GetLastError() == ERROR_PIPE_CONNECTED,
                }
            };
            let current = listening;
            // A próxima instância existe antes de atender esta.
            listening = create_pipe(false, &sddl)?;
            if connected && client_is_agent(current, install_dir.as_deref()) {
                serve_client(current, &mut injector, &mut desktop);
            }
            // SAFETY: encerra a conexão e fecha o handle possuído.
            unsafe {
                let _ = DisconnectNamedPipe(current);
                let _ = CloseHandle(current);
            }
        }
    }

    fn serve_client(pipe: HANDLE, injector: &mut WindowsInjector, desktop: &mut DesktopFollower) {
        let mut asm = FrameAssembler::default();
        let mut buf = [0u8; 4096];
        let text = &mut Vec::<u16>::new();
        loop {
            let mut read = 0u32;
            // SAFETY: `buf` e `read` vivem durante a chamada (síncrona).
            let ok = unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut read), None) };
            if ok.is_err() || read == 0 {
                return;
            }
            let frames = match asm.push(&buf[..read as usize]) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("frame inválido, derrubando o cliente: {e}");
                    return;
                }
            };
            if frames.is_empty() {
                continue;
            }
            // Um lote por leitura: o desktop de input é conferido uma vez antes dele.
            desktop.follow();
            for frame in frames {
                match frame {
                    InjectFrame::Event(ev) => {
                        flush_text(injector, text);
                        if let Err(e) = injector.inject(&ev) {
                            tracing::debug!("injeção falhou: {e}");
                        }
                    }
                    InjectFrame::TextUnit(u) => text.push(u),
                }
            }
            // Par substituto cortado no fim da leitura espera o resto na próxima.
            if !text.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
                flush_text(injector, text);
            }
        }
    }
}

#[cfg(windows)]
fn flush_text(injector: &mut crate::inject::WindowsInjector, text: &mut Vec<u16>) {
    use relay_core::platform::InputInjector;
    if text.is_empty() {
        return;
    }
    let s = String::from_utf16_lossy(text);
    text.clear();
    if let Err(e) = injector.type_text(&s) {
        tracing::debug!("type_text falhou: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sddl_restricts_to_system_and_session_user() {
        assert_eq!(
            pipe_sddl(Some("S-1-5-21-1-2-3-1001")),
            "D:P(A;;GA;;;SY)(A;;0x100002;;;S-1-5-21-1-2-3-1001)"
        );
        assert_eq!(pipe_sddl(None), "D:P(A;;GA;;;SY)");
        assert!(!pipe_sddl(None).contains("WD") && !pipe_sddl(None).contains("BU"));
    }

    #[test]
    fn backoff_doubles_up_to_cap_and_resets() {
        let mut b = Backoff::default();
        let ds: Vec<_> = (0..8).map(|_| b.next_delay().as_millis()).collect();
        assert_eq!(ds, [250, 500, 1000, 2000, 4000, 8000, 10_000, 10_000]);
        b.reset();
        assert_eq!(b.next_delay(), Backoff::BASE);
        assert_eq!(Backoff::delay_for(u32::MAX), Backoff::MAX);
    }

    #[test]
    fn relaunch_decision() {
        assert!(needs_relaunch(None, 1));
        assert!(!needs_relaunch(Some(1), 1));
        assert!(needs_relaunch(Some(1), 2));
        assert!(!needs_relaunch(None, NO_SESSION));
        assert!(!needs_relaunch(Some(1), NO_SESSION));
    }

    #[test]
    fn helper_command_line_quotes_path() {
        assert_eq!(
            helper_command_line(r"C:\Program Files\Relay\relay-service.exe"),
            r#""C:\Program Files\Relay\relay-service.exe" --session-helper"#
        );
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/Program Files")]
    }

    #[test]
    fn client_verification() {
        let dir = Path::new("/Program Files/Relay Agent");
        let ok = Path::new("/program files/relay agent/Relay-Windows-Agent.EXE");
        assert_eq!(verify_client_image(ok, dir, &roots()), Ok(()));
        let ok2 = Path::new("/Program Files/Relay Agent/Relay Agent.exe");
        assert_eq!(verify_client_image(ok2, dir, &roots()), Ok(()));
        let other_dir = Path::new("/Users/x/AppData/relay-windows-agent.exe");
        assert_eq!(
            verify_client_image(other_dir, dir, &roots()),
            Err(ClientDenied::WrongDirectory)
        );
        let sub = Path::new("/Program Files/Relay Agent/sub/relay-windows-agent.exe");
        assert_eq!(
            verify_client_image(sub, dir, &roots()),
            Err(ClientDenied::WrongDirectory)
        );
        let other_exe = Path::new("/Program Files/Relay Agent/evil.exe");
        assert_eq!(
            verify_client_image(other_exe, dir, &roots()),
            Err(ClientDenied::WrongImage)
        );
    }

    #[test]
    fn client_verification_requires_trusted_install_dir() {
        let dir = Path::new("/Users/x/Downloads/relay");
        let img = Path::new("/Users/x/Downloads/relay/relay-windows-agent.exe");
        assert_eq!(
            verify_client_image(img, dir, &roots()),
            Err(ClientDenied::InstallDirUntrusted)
        );
        // A raiz em si e prefixos de string parecidos não valem.
        let root = Path::new("/Program Files");
        assert_eq!(
            verify_client_image(&root.join("relay-windows-agent.exe"), root, &roots()),
            Err(ClientDenied::InstallDirUntrusted)
        );
        let sibling = Path::new("/Program Files (x86)/Relay");
        assert_eq!(
            verify_client_image(&sibling.join("relay-windows-agent.exe"), sibling, &roots()),
            Err(ClientDenied::InstallDirUntrusted)
        );
    }
}

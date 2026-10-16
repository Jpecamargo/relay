//! Serviço SYSTEM (cfg(windows)): atende o SCM e mantém um helper vivo na sessão ativa.

use std::ffi::OsString;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use relay_platform_windows::service::{
    Backoff, HelperProcess, SERVICE_NAME, STABLE_AFTER, active_console_session, launch_helper,
    needs_relaunch,
};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

define_windows_service!(ffi_service_main, service_main);

enum Event {
    Stop,
    SessionChange,
}

/// Bloqueia até o SCM encerrar o serviço.
pub fn run() -> Result<(), windows_service::Error> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = supervise() {
        tracing::error!("serviço terminou com erro: {e}");
    }
}

fn status(state: ServiceState, accept: ServiceControlAccept, wait: Duration) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: wait,
        process_id: None,
    }
}

fn supervise() -> windows_service::Result<()> {
    let (tx, rx) = mpsc::channel();
    let handler = move |control| match control {
        ServiceControl::Stop => {
            let _ = tx.send(Event::Stop);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::SessionChange(_) => {
            let _ = tx.send(Event::SessionChange);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let handle = service_control_handler::register(SERVICE_NAME, handler)?;
    let accept = ServiceControlAccept::STOP | ServiceControlAccept::SESSION_CHANGE;
    handle.set_service_status(status(ServiceState::Running, accept, Duration::ZERO))?;

    let mut helper: Option<HelperProcess> = None;
    let mut backoff = Backoff::default();
    let mut next_launch = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Event::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Event::SessionChange) | Err(RecvTimeoutError::Timeout) => {}
        }
        // Queda do helper: recuo exponencial; helper estável zera o recuo.
        if let Some(h) = &helper
            && !h.is_running()
        {
            if h.started.elapsed() >= STABLE_AFTER {
                backoff.reset();
            }
            next_launch = Instant::now() + backoff.next_delay();
            helper = None;
        }
        let Some(active) = active_console_session() else {
            // Sem sessão de console (troca em andamento): encerra o helper órfão.
            if let Some(h) = helper.take() {
                h.terminate();
            }
            continue;
        };
        if let Some(h) = &helper
            && needs_relaunch(Some(h.session), active)
        {
            h.terminate();
            helper = None;
            backoff.reset();
            next_launch = Instant::now();
        }
        if helper.is_none() && Instant::now() >= next_launch {
            match launch_helper(active) {
                Ok(h) => helper = Some(h),
                Err(e) => {
                    tracing::warn!("falha ao lançar o helper na sessão {active}: {e}");
                    next_launch = Instant::now() + backoff.next_delay();
                }
            }
        }
    }

    if let Some(h) = helper.take() {
        h.terminate();
    }
    handle.set_service_status(status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        Duration::ZERO,
    ))
}

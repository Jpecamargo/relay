//! dono: WI2
//! Executor dos `RemoteCommand` no Windows. O roteamento é genérico sobre [`SystemOps`] (testado no
//! Mac com mock); [`WindowsOps`] liga cada operação aos módulos de `relay-platform-windows`.

use std::sync::Mutex;

use relay_core::platform::{CommandExecutor, InputInjector, PlatformError};
use relay_core::types::{
    AudioDevice, CommandOutput, CommandResult, InputEvent, MediaAction, MediaKey, PowerAction,
    RemoteCommand, SystemMetrics, error_code,
};
use relay_platform_windows::audio_endpoints::{self, EndpointKind};
use relay_platform_windows::inject::WindowsInjector;
use relay_platform_windows::{metrics, power, screen_capture, shell};

/// Operações do sistema que o executor usa. Uma por comando; erros são `PlatformError`.
pub trait SystemOps: Send {
    fn set_volume(&mut self, level: f32) -> Result<(), PlatformError>;
    fn set_mute(&mut self, muted: bool) -> Result<(), PlatformError>;
    fn list_output_devices(&mut self) -> Result<Vec<AudioDevice>, PlatformError>;
    fn set_output_device(&mut self, id: &str) -> Result<(), PlatformError>;
    /// Pressiona e solta a tecla de mídia (VK).
    fn media(&mut self, key: MediaKey) -> Result<(), PlatformError>;
    fn power(&mut self, action: PowerAction) -> Result<(), PlatformError>;
    fn open_target(&mut self, target: &str) -> Result<(), PlatformError>;
    fn capture_screen(&mut self, display_id: &str) -> Result<Vec<u8>, PlatformError>;
    fn metrics(&mut self) -> SystemMetrics;
}

/// Destino dos PNGs de `CaptureScreen`: viram `ClipboardOffer` (D7), nunca vão no fio como
/// `CommandResult`.
pub type PngSink = Box<dyn FnMut(Vec<u8>) + Send>;

pub struct CommandRunner<O: SystemOps> {
    ops: O,
    png: PngSink,
}

impl<O: SystemOps> CommandRunner<O> {
    pub fn new(ops: O, png: PngSink) -> Self {
        Self { ops, png }
    }

    fn run(&mut self, cmd: &RemoteCommand) -> Result<Option<CommandOutput>, PlatformError> {
        match cmd {
            RemoteCommand::SetVolume { level } => self.ops.set_volume(*level).map(|()| None),
            RemoteCommand::SetMute { muted } => self.ops.set_mute(*muted).map(|()| None),
            RemoteCommand::ListOutputDevices => self
                .ops
                .list_output_devices()
                .map(|d| Some(CommandOutput::Devices(d))),
            RemoteCommand::SetOutputDevice { id } => self.ops.set_output_device(id).map(|()| None),
            RemoteCommand::Media { action } => {
                let key = match action {
                    MediaAction::PlayPause => MediaKey::PlayPause,
                    MediaAction::Next => MediaKey::Next,
                    MediaAction::Previous => MediaKey::Previous,
                };
                self.ops.media(key).map(|()| None)
            }
            RemoteCommand::Power { action, confirm } => {
                let destructive = matches!(action, PowerAction::Restart | PowerAction::Shutdown);
                if destructive && !confirm {
                    return Err(PlatformError::Platform(
                        error_code::CONFIRMATION_REQUIRED.into(),
                    ));
                }
                self.ops.power(*action).map(|()| None)
            }
            RemoteCommand::CaptureScreen { display_id } => {
                let png = self.ops.capture_screen(display_id)?;
                (self.png)(png);
                Ok(None)
            }
            RemoteCommand::OpenTarget { target } => self.ops.open_target(target).map(|()| None),
            // O Receiver digita pelo injetor antes de chegar ao executor.
            RemoteCommand::TypeText { .. } => Err(PlatformError::Unsupported),
            RemoteCommand::GetMetrics => Ok(Some(CommandOutput::Metrics(self.ops.metrics()))),
        }
    }
}

impl<O: SystemOps> CommandExecutor for CommandRunner<O> {
    fn execute(&mut self, id: u32, command: &RemoteCommand) -> CommandResult {
        match self.run(command) {
            Ok(output) => CommandResult::ok(id, output),
            Err(PlatformError::NotImplemented | PlatformError::Unsupported) => {
                CommandResult::err(id, error_code::UNSUPPORTED)
            }
            Err(PlatformError::Platform(msg)) => CommandResult::err(id, msg),
        }
    }
}

/// Capturas de tela aguardando virar `ClipboardOffer` (WI3 esvazia com [`take_captures`] e envia
/// pelo canal de clipboard).
static CAPTURES: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// Remove e devolve os PNGs capturados desde a última chamada.
pub fn take_captures() -> Vec<Vec<u8>> {
    CAPTURES
        .lock()
        .map(|mut c| std::mem::take(&mut *c))
        .unwrap_or_default()
}

/// Operações reais, via módulos de plataforma.
#[derive(Default)]
pub struct WindowsOps {
    injector: WindowsInjector,
}

impl SystemOps for WindowsOps {
    fn set_volume(&mut self, level: f32) -> Result<(), PlatformError> {
        audio_endpoints::set_volume(EndpointKind::Render, level)
    }
    fn set_mute(&mut self, muted: bool) -> Result<(), PlatformError> {
        audio_endpoints::set_mute(EndpointKind::Render, muted)
    }
    fn list_output_devices(&mut self) -> Result<Vec<AudioDevice>, PlatformError> {
        audio_endpoints::list_all().map(|d| d.outputs)
    }
    fn set_output_device(&mut self, id: &str) -> Result<(), PlatformError> {
        audio_endpoints::set_default_endpoint(id)
    }
    fn media(&mut self, key: MediaKey) -> Result<(), PlatformError> {
        for down in [true, false] {
            self.injector
                .inject(&InputEvent::Media { key, down })
                .map_err(|e| PlatformError::Platform(e.to_string()))?;
        }
        Ok(())
    }
    fn power(&mut self, action: PowerAction) -> Result<(), PlatformError> {
        match action {
            PowerAction::Lock => power::lock(),
            PowerAction::Suspend => power::suspend(),
            PowerAction::Restart => power::restart(),
            PowerAction::Shutdown => power::shutdown(),
        }
    }
    fn open_target(&mut self, target: &str) -> Result<(), PlatformError> {
        shell::open_target(target)
    }
    fn capture_screen(&mut self, display_id: &str) -> Result<Vec<u8>, PlatformError> {
        screen_capture::capture_monitor_png(display_id)
    }
    fn metrics(&mut self) -> SystemMetrics {
        // Volume e mudo atuais para o painel do Mac (ausentes se o Core Audio falhar).
        SystemMetrics {
            volume: audio_endpoints::get_volume(EndpointKind::Render).ok(),
            muted: audio_endpoints::get_mute(EndpointKind::Render).ok(),
            ..metrics::collect_metrics()
        }
    }
}

/// Executor de produção; os PNGs vão para a fila de [`take_captures`].
pub fn windows_executor() -> Box<dyn CommandExecutor> {
    let sink: PngSink = Box::new(|png| {
        if let Ok(mut c) = CAPTURES.lock() {
            c.push(png);
        }
    });
    Box::new(CommandRunner::new(WindowsOps::default(), sink))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    type Calls = Arc<Mutex<Vec<String>>>;

    struct Mock {
        calls: Calls,
        fail: bool,
    }

    impl Mock {
        fn rec(&self, s: String) -> Result<(), PlatformError> {
            self.calls.lock().unwrap().push(s);
            if self.fail {
                Err(PlatformError::Platform("falhou".into()))
            } else {
                Ok(())
            }
        }
    }

    impl SystemOps for Mock {
        fn set_volume(&mut self, level: f32) -> Result<(), PlatformError> {
            self.rec(format!("set_volume {level}"))
        }
        fn set_mute(&mut self, muted: bool) -> Result<(), PlatformError> {
            self.rec(format!("set_mute {muted}"))
        }
        fn list_output_devices(&mut self) -> Result<Vec<AudioDevice>, PlatformError> {
            self.rec("list".into())?;
            Ok(vec![AudioDevice {
                id: "d1".into(),
                name: "Alto-falantes".into(),
                is_default: true,
            }])
        }
        fn set_output_device(&mut self, id: &str) -> Result<(), PlatformError> {
            self.rec(format!("set_output {id}"))
        }
        fn media(&mut self, key: MediaKey) -> Result<(), PlatformError> {
            self.rec(format!("media {key:?}"))
        }
        fn power(&mut self, action: PowerAction) -> Result<(), PlatformError> {
            self.rec(format!("power {action:?}"))
        }
        fn open_target(&mut self, target: &str) -> Result<(), PlatformError> {
            self.rec(format!("open {target}"))
        }
        fn capture_screen(&mut self, display_id: &str) -> Result<Vec<u8>, PlatformError> {
            self.rec(format!("capture {display_id}"))?;
            Ok(vec![137, 80, 78, 71])
        }
        fn metrics(&mut self) -> SystemMetrics {
            self.calls.lock().unwrap().push("metrics".into());
            SystemMetrics {
                cpu_pct: 12.0,
                ..SystemMetrics::default()
            }
        }
    }

    struct Harness {
        ex: CommandRunner<Mock>,
        calls: Calls,
        pngs: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    fn harness(fail: bool) -> Harness {
        let calls = Calls::default();
        let pngs = Arc::new(Mutex::new(Vec::new()));
        let p = pngs.clone();
        let ex = CommandRunner::new(
            Mock {
                calls: calls.clone(),
                fail,
            },
            Box::new(move |png| p.lock().unwrap().push(png)),
        );
        Harness { ex, calls, pngs }
    }

    fn run(cmd: RemoteCommand) -> (CommandResult, Vec<String>) {
        let mut h = harness(false);
        let r = h.ex.execute(5, &cmd);
        let c = h.calls.lock().unwrap().clone();
        (r, c)
    }

    #[test]
    fn set_volume() {
        let (r, c) = run(RemoteCommand::SetVolume { level: 0.5 });
        assert_eq!(r, CommandResult::ok(5, None));
        assert_eq!(c, ["set_volume 0.5"]);
    }

    #[test]
    fn set_mute() {
        let (r, c) = run(RemoteCommand::SetMute { muted: true });
        assert_eq!(r, CommandResult::ok(5, None));
        assert_eq!(c, ["set_mute true"]);
    }

    #[test]
    fn list_output_devices() {
        let (r, c) = run(RemoteCommand::ListOutputDevices);
        assert!(
            matches!(&r.output, Some(CommandOutput::Devices(d)) if d.len() == 1 && d[0].id == "d1")
        );
        assert_eq!(c, ["list"]);
    }

    #[test]
    fn set_output_device() {
        let (r, c) = run(RemoteCommand::SetOutputDevice { id: "d2".into() });
        assert_eq!(r, CommandResult::ok(5, None));
        assert_eq!(c, ["set_output d2"]);
    }

    #[test]
    fn media_maps_each_action() {
        for (action, expected) in [
            (MediaAction::PlayPause, "media PlayPause"),
            (MediaAction::Next, "media Next"),
            (MediaAction::Previous, "media Previous"),
        ] {
            let (r, c) = run(RemoteCommand::Media { action });
            assert_eq!(r, CommandResult::ok(5, None));
            assert_eq!(c, [expected]);
        }
    }

    #[test]
    fn power_lock_and_suspend_need_no_confirmation() {
        for (action, expected) in [
            (PowerAction::Lock, "power Lock"),
            (PowerAction::Suspend, "power Suspend"),
        ] {
            let (r, c) = run(RemoteCommand::Power {
                action,
                confirm: false,
            });
            assert_eq!(r, CommandResult::ok(5, None));
            assert_eq!(c, [expected]);
        }
    }

    #[test]
    fn power_destructive_requires_confirm() {
        for action in [PowerAction::Restart, PowerAction::Shutdown] {
            let (r, c) = run(RemoteCommand::Power {
                action,
                confirm: false,
            });
            assert_eq!(r, CommandResult::err(5, error_code::CONFIRMATION_REQUIRED));
            assert!(c.is_empty());
            let (r, c) = run(RemoteCommand::Power {
                action,
                confirm: true,
            });
            assert_eq!(r, CommandResult::ok(5, None));
            assert_eq!(c, [format!("power {action:?}")]);
        }
    }

    #[test]
    fn capture_screen_goes_to_sink_not_to_wire() {
        let mut h = harness(false);
        let r = h.ex.execute(
            5,
            &RemoteCommand::CaptureScreen {
                display_id: "MONITOR\\X".into(),
            },
        );
        assert_eq!(r, CommandResult::ok(5, None));
        assert_eq!(*h.calls.lock().unwrap(), ["capture MONITOR\\X"]);
        assert_eq!(*h.pngs.lock().unwrap(), [vec![137, 80, 78, 71]]);
    }

    #[test]
    fn open_target() {
        let (r, c) = run(RemoteCommand::OpenTarget {
            target: "https://exemplo.com".into(),
        });
        assert_eq!(r, CommandResult::ok(5, None));
        assert_eq!(c, ["open https://exemplo.com"]);
    }

    #[test]
    fn get_metrics() {
        let (r, c) = run(RemoteCommand::GetMetrics);
        assert!(matches!(&r.output, Some(CommandOutput::Metrics(m)) if m.cpu_pct == 12.0));
        assert_eq!(c, ["metrics"]);
    }

    #[test]
    fn platform_failure_becomes_error_result() {
        let mut h = harness(true);
        let r = h.ex.execute(9, &RemoteCommand::SetMute { muted: false });
        assert_eq!(r, CommandResult::err(9, "falhou"));
        let r = h.ex.execute(
            10,
            &RemoteCommand::CaptureScreen {
                display_id: "x".into(),
            },
        );
        assert!(!r.ok);
        assert!(h.pngs.lock().unwrap().is_empty());
    }
}

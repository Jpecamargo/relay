//! dono: W1
//! Injeção via SendInput. A montagem dos eventos é pura (`inject_map.rs`); aqui só a chamada Win32.

use relay_core::platform::{InjectError, InputInjector};
use relay_core::types::InputEvent;

#[derive(Debug, Default)]
pub struct WindowsInjector {}

#[cfg(windows)]
mod win {
    use super::InjectError;
    use crate::inject_map::{RawInput, VirtualRect};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
        MOUSE_EVENT_FLAGS, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    pub fn virtual_rect() -> VirtualRect {
        // SAFETY: GetSystemMetrics não tem pré-condições.
        unsafe {
            VirtualRect {
                left: GetSystemMetrics(SM_XVIRTUALSCREEN),
                top: GetSystemMetrics(SM_YVIRTUALSCREEN),
                width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
                height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
            }
        }
    }

    fn to_input(raw: &RawInput) -> INPUT {
        match *raw {
            RawInput::Mouse(m) => INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: m.dx,
                        dy: m.dy,
                        mouseData: m.data,
                        dwFlags: MOUSE_EVENT_FLAGS(m.flags),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            RawInput::Key(k) => INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(k.vk),
                        wScan: k.scan,
                        dwFlags: KEYBD_EVENT_FLAGS(k.flags),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        }
    }

    /// Uma chamada de SendInput para as entradas dadas (até 3, array na pilha).
    pub fn send(raws: &[RawInput]) -> Result<(), InjectError> {
        let blank = INPUT::default();
        let mut buf = [blank; 3];
        let n = raws.len().min(3);
        for (slot, raw) in buf.iter_mut().zip(&raws[..n]) {
            *slot = to_input(raw);
        }
        send_slice(&buf[..n])
    }

    pub fn send_slice(inputs: &[INPUT]) -> Result<(), InjectError> {
        if inputs.is_empty() {
            return Ok(());
        }
        // SAFETY: o slice é válido e `cbSize` é o tamanho de INPUT.
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(InjectError::Platform(format!(
                "SendInput injetou {sent} de {} (UIPI/desktop seguro?)",
                inputs.len()
            )))
        }
    }

    pub fn send_many(raws: &[RawInput]) -> Result<(), InjectError> {
        let inputs: Vec<INPUT> = raws.iter().map(to_input).collect();
        send_slice(&inputs)
    }
}

impl InputInjector for WindowsInjector {
    #[cfg(windows)]
    fn inject(&mut self, event: &InputEvent) -> Result<(), InjectError> {
        let rect = match event {
            InputEvent::MouseAbs { .. } => win::virtual_rect(),
            _ => crate::inject_map::VirtualRect {
                left: 0,
                top: 0,
                width: 0,
                height: 0,
            },
        };
        let batch = crate::inject_map::event_inputs(event, rect);
        win::send(batch.as_slice())
    }

    #[cfg(not(windows))]
    fn inject(&mut self, _event: &InputEvent) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }

    #[cfg(windows)]
    fn type_text(&mut self, text: &str) -> Result<(), InjectError> {
        win::send_many(&crate::inject_map::text_inputs(text))
    }

    #[cfg(not(windows))]
    fn type_text(&mut self, _text: &str) -> Result<(), InjectError> {
        Err(InjectError::NotImplemented)
    }
}

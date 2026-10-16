//! dono: R1
//! Sentido inverso no Mac (D24, D25): o teclado e o mouse do PC controlam o Mac. O Mac vira
//! `Receiver` (com o `MacInjector` e o `MacCursor` já usados pelo Controller) enquanto o agente
//! tem o foco; o `Controller` do Mac fica pausado nesse período, então só um lado controla.
//!
//! Tudo roda na thread `relay-core` (engine.rs), como o Controller: este módulo só empacota o
//! `Receiver` e a regra de arbitragem.

use std::time::{Duration, Instant};

use relay_core::engine::Receiver;
use relay_core::types::{Config, DisplayInfo, InputEvent, LayoutProfile, Machine, Message};

use super::platform::{Platform, Shared};

/// Prazo para o `FocusReturn` de confirmação do PC depois de uma recusa.
const REFUSAL_ACK: Duration = Duration::from_secs(2);

pub(crate) struct MacReverse<P: Platform> {
    rx: Receiver<Shared<P::Injector>, Shared<P::Cursor>>,
    /// Quando o último `FocusEnter` foi recusado: o PC confirma com um `FocusReturn` que o
    /// Controller do Mac não pode tomar por um pedido de retorno.
    refused_at: Option<Instant>,
}

impl<P: Platform> MacReverse<P> {
    pub fn new(
        config: &Config,
        local: Vec<DisplayInfo>,
        injector: Shared<P::Injector>,
        cursor: Shared<P::Cursor>,
        profile: Option<LayoutProfile>,
    ) -> Self {
        let mut rx = Receiver::new(config, local, injector, cursor);
        rx.set_machine(Machine::Mac);
        if let Some(p) = profile {
            rx.set_profile(p);
        }
        Self {
            rx,
            refused_at: None,
        }
    }

    /// `true` enquanto o PC controla o Mac.
    pub fn is_active(&self) -> bool {
        self.rx.is_active()
    }

    pub fn set_local_displays(&mut self, displays: Vec<DisplayInfo>) {
        self.rx.set_local_displays(displays);
    }

    pub fn set_profile(&mut self, profile: LayoutProfile) {
        self.rx.set_profile(profile);
    }

    /// O PC largou o Mac (ou a sessão caiu): solta as teclas que ele deixou pressionadas.
    pub fn focus_lost(&mut self) {
        self.rx.focus_lost();
    }

    pub fn tick(&mut self, now: Instant) {
        if self.rx.is_active() {
            self.rx.tick(now);
        }
    }

    /// Movimento/rolagem do canal rápido. Devolve o que enviar ao PC (`FocusReturn`).
    pub fn on_input(&mut self, event: InputEvent, now: Instant) -> Vec<Message> {
        self.rx.on_input(event, now)
    }

    /// `true` se este `FocusReturn` do PC só confirma uma recusa nossa (e deve ser ignorado).
    pub fn take_refusal_ack(&mut self, now: Instant) -> bool {
        match self.refused_at.take() {
            Some(t) => now.saturating_duration_since(t) < REFUSAL_ACK,
            None => false,
        }
    }

    /// `Input`, `FocusEnter`, `FocusReturn` e `Displays` vindos do PC. O `FocusEnter` só é aceito
    /// se `mac_controls` for `false`: com o Mac no controle do PC, o PC não assume (D24) e a
    /// resposta é um `FocusReturn`, que faz o Controller do agente voltar.
    pub fn on_message(&mut self, msg: Message, mac_controls: bool, now: Instant) -> Vec<Message> {
        if let Message::FocusEnter {
            display_id,
            entry_px,
        } = &msg
            && mac_controls
        {
            self.refused_at = Some(now);
            return vec![Message::FocusReturn {
                display_id: display_id.clone(),
                point_px: *entry_px,
            }];
        }
        self.rx.on_message(msg, now)
    }
}

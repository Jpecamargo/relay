//! dono: C10
//! Correlação de `Command{id}` com `CommandResult{id}`, com timeout de 5 s.

use std::time::{Duration, Instant};

use crate::types::{CommandResult, Message, RemoteCommand, error_code};

/// Sem resposta neste prazo, o resultado vira erro `timeout`.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct CommandDispatcher {
    next_id: u32,
    pending: Vec<(u32, Instant)>,
    done: Vec<CommandResult>,
}

impl CommandDispatcher {
    /// Reserva um `id` para um comando enviado agora.
    pub fn register(&mut self) -> u32 {
        self.register_at(Instant::now())
    }

    pub fn register_at(&mut self, now: Instant) -> u32 {
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.push((self.next_id, now));
        self.next_id
    }

    /// Registra e monta a `Message::Command` a enviar.
    pub fn command_message(&mut self, cmd: RemoteCommand, now: Instant) -> Message {
        Message::Command {
            id: self.register_at(now),
            cmd,
        }
    }

    /// Resposta do Windows. `false` se o `id` não está pendente (duplicada ou já expirada).
    pub fn on_result(&mut self, result: CommandResult) -> bool {
        match self.pending.iter().position(|(id, _)| *id == result.id) {
            Some(i) => {
                self.pending.remove(i);
                self.done.push(result);
                true
            }
            None => false,
        }
    }

    /// Expira os pendentes vencidos (viram `CommandResult` com erro `timeout`) e entrega os
    /// resultados prontos, na ordem em que ficaram prontos.
    pub fn poll(&mut self, now: Instant) -> Vec<CommandResult> {
        let mut i = 0;
        while i < self.pending.len() {
            let (id, since) = self.pending[i];
            if now.saturating_duration_since(since) >= COMMAND_TIMEOUT {
                self.pending.remove(i);
                self.done.push(CommandResult::err(id, error_code::TIMEOUT));
            } else {
                i += 1;
            }
        }
        std::mem::take(&mut self.done)
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CommandOutput;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn correlaciona_resultado_com_id() {
        let t = Instant::now();
        let mut d = CommandDispatcher::default();
        let a = d.register_at(t);
        let b = d.register_at(t);
        assert_ne!(a, b);
        assert!(d.on_result(CommandResult::ok(b, Some(CommandOutput::Text("b".into())))));
        assert!(d.on_result(CommandResult::ok(a, None)));
        let r = d.poll(t + ms(10));
        assert_eq!(r.iter().map(|r| r.id).collect::<Vec<_>>(), vec![b, a]);
        assert!(r.iter().all(|r| r.ok));
        assert_eq!(d.pending(), 0);
        // resposta repetida ou de id desconhecido é ignorada
        assert!(!d.on_result(CommandResult::ok(a, None)));
        assert!(d.poll(t + ms(20)).is_empty());
    }

    #[test]
    fn sem_resposta_em_5s_vira_timeout() {
        let t = Instant::now();
        let mut d = CommandDispatcher::default();
        let msg = d.command_message(RemoteCommand::GetMetrics, t);
        let Message::Command { id, .. } = msg else {
            panic!("{msg:?}")
        };
        assert!(d.poll(t + ms(4999)).is_empty());
        let r = d.poll(t + ms(5000));
        assert_eq!(r, vec![CommandResult::err(id, "timeout")]);
        // resposta tardia não ressuscita o comando
        assert!(!d.on_result(CommandResult::ok(id, None)));
        assert!(d.poll(t + ms(6000)).is_empty());
    }

    #[test]
    fn timeout_de_um_nao_afeta_o_outro() {
        let t = Instant::now();
        let mut d = CommandDispatcher::default();
        let a = d.register_at(t);
        let b = d.register_at(t + ms(3000));
        assert!(d.on_result(CommandResult::ok(b, None)));
        let r = d.poll(t + ms(5500));
        assert_eq!(r.len(), 2);
        assert!(
            r.iter()
                .any(|r| r.id == a && r.error.as_deref() == Some("timeout"))
        );
        assert!(r.iter().any(|r| r.id == b && r.ok));
    }
}

//! dono: C11a
//! Sincronização de clipboard no core: oferta e blocos pelo canal confiável, limite de tamanho,
//! anti-eco e modos.
//!
//! O conteúdo é serializado (bincode) e enviado como `ClipboardOffer` seguido de
//! `ClipboardChunk`s de `CHUNK_SIZE`. A remontagem confere ordem e completude e descarta a oferta
//! incompleta após `OFFER_TIMEOUT`. O hash do último conteúdo aplicado do remoto evita o eco.

use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::types::consts::CHUNK_SIZE;
use crate::types::{ClipboardConfig, ClipboardContent, ClipboardMode, Message};

/// Prazo para completar uma oferta recebida.
pub const OFFER_TIMEOUT: Duration = Duration::from_secs(10);

type Hash = [u8; 32];

#[derive(Debug)]
struct Incoming {
    offer_id: u32,
    total: usize,
    next_index: u32,
    buf: Vec<u8>,
    started: Instant,
}

#[derive(Debug)]
pub struct ClipboardSync {
    config: ClipboardConfig,
    next_offer_id: u32,
    incoming: Option<Incoming>,
    /// Hash do último conteúdo aplicado a partir do remoto (anti-eco).
    last_applied: Option<Hash>,
    /// Hash do último conteúdo enviado (evita reenviar o mesmo no `on_switch`).
    last_sent: Option<Hash>,
}

fn encode(content: &ClipboardContent) -> Option<Vec<u8>> {
    bincode::serde::encode_to_vec(content, bincode::config::standard()).ok()
}

fn hash(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}

impl ClipboardSync {
    pub fn new(config: ClipboardConfig) -> Self {
        Self {
            config,
            next_offer_id: 1,
            incoming: None,
            last_applied: None,
            last_sent: None,
        }
    }

    pub fn set_config(&mut self, config: ClipboardConfig) {
        self.config = config;
    }

    /// Conteúdo local novo: em `OnCopy` devolve as mensagens a enviar (oferta + blocos, ou
    /// `ClipboardSkipped`); em `OnSwitch` nada sai até `on_switch`/`send_now`.
    pub fn on_local_change(&mut self, content: ClipboardContent) -> Vec<Message> {
        if self.config.mode != ClipboardMode::OnCopy {
            return Vec::new();
        }
        self.offer(content, false)
    }

    /// Troca de foco entre máquinas: em `OnSwitch` oferece o conteúdo atual do clipboard.
    pub fn on_switch(&mut self, current: Option<ClipboardContent>) -> Vec<Message> {
        if self.config.mode != ClipboardMode::OnSwitch {
            return Vec::new();
        }
        match current {
            Some(content) => self.offer(content, false),
            None => Vec::new(),
        }
    }

    /// "Enviar agora": força a oferta do conteúdo atual, em qualquer modo.
    pub fn send_now(&mut self, current: Option<ClipboardContent>) -> Vec<Message> {
        match current {
            Some(content) => self.offer(content, true),
            None => Vec::new(),
        }
    }

    fn offer(&mut self, content: ClipboardContent, force: bool) -> Vec<Message> {
        if !self.config.enabled || !self.config.formats.contains(&content.format()) {
            return Vec::new();
        }
        let Some(bytes) = encode(&content) else {
            return Vec::new();
        };
        if bytes.len() as u64 > self.config.max_bytes {
            return vec![Message::ClipboardSkipped {
                reason: "too_large".into(),
            }];
        }
        let h = hash(&bytes);
        if !force && (self.last_applied == Some(h) || self.last_sent == Some(h)) {
            return Vec::new();
        }
        self.last_sent = Some(h);
        let offer_id = self.next_offer_id;
        self.next_offer_id = self.next_offer_id.wrapping_add(1);
        let mut out = Vec::with_capacity(1 + bytes.len().div_ceil(CHUNK_SIZE));
        out.push(Message::ClipboardOffer {
            offer_id,
            formats: vec![content.format()],
            total: bytes.len() as u64,
        });
        for (i, chunk) in bytes.chunks(CHUNK_SIZE).enumerate() {
            out.push(Message::ClipboardChunk {
                offer_id,
                index: i as u32,
                data: chunk.to_vec(),
            });
        }
        out
    }

    /// Mensagem de clipboard recebida; devolve o conteúdo remontado quando completa.
    pub fn on_message(&mut self, msg: Message) -> Option<ClipboardContent> {
        self.on_message_at(msg, Instant::now())
    }

    /// Como `on_message`, com o relógio injetado.
    pub fn on_message_at(&mut self, msg: Message, now: Instant) -> Option<ClipboardContent> {
        self.expire(now);
        match msg {
            Message::ClipboardOffer {
                offer_id, total, ..
            } => {
                self.incoming = None;
                if !self.config.enabled || total == 0 || total > self.config.max_bytes {
                    return None;
                }
                self.incoming = Some(Incoming {
                    offer_id,
                    total: total as usize,
                    next_index: 0,
                    buf: Vec::with_capacity((total as usize).min(CHUNK_SIZE * 4)),
                    started: now,
                });
                None
            }
            Message::ClipboardChunk {
                offer_id,
                index,
                data,
            } => self.on_chunk(offer_id, index, data),
            _ => None,
        }
    }

    fn on_chunk(&mut self, offer_id: u32, index: u32, data: Vec<u8>) -> Option<ClipboardContent> {
        let inc = self.incoming.as_mut()?;
        if inc.offer_id != offer_id {
            return None;
        }
        if index != inc.next_index
            || data.len() > CHUNK_SIZE
            || inc.buf.len() + data.len() > inc.total
        {
            self.incoming = None;
            return None;
        }
        inc.buf.extend_from_slice(&data);
        inc.next_index += 1;
        if inc.buf.len() < inc.total {
            return None;
        }
        let inc = self.incoming.take()?;
        let (content, _): (ClipboardContent, usize) =
            bincode::serde::decode_from_slice(&inc.buf, bincode::config::standard()).ok()?;
        if !self.config.formats.contains(&content.format()) {
            return None;
        }
        self.last_applied = Some(hash(&inc.buf));
        Some(content)
    }

    /// Descarta a oferta incompleta mais velha que `OFFER_TIMEOUT`.
    pub fn expire(&mut self, now: Instant) {
        if let Some(inc) = &self.incoming {
            if now.saturating_duration_since(inc.started) > OFFER_TIMEOUT {
                self.incoming = None;
            }
        }
    }

    /// Há oferta remota em andamento.
    pub fn has_pending_offer(&self) -> bool {
        self.incoming.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClipboardFormat, FileEntry};

    fn cfg(mode: ClipboardMode) -> ClipboardConfig {
        ClipboardConfig {
            mode,
            ..ClipboardConfig::default()
        }
    }

    fn deliver(rx: &mut ClipboardSync, msgs: Vec<Message>) -> Option<ClipboardContent> {
        msgs.into_iter()
            .fold(None, |acc, m| rx.on_message(m).or(acc))
    }

    #[test]
    fn all_formats_round_trip() {
        let items = vec![
            ClipboardContent::Text("olá".into()),
            ClipboardContent::Html("<b>x</b>".into()),
            ClipboardContent::Rtf(vec![1, 2, 3]),
            ClipboardContent::Png(vec![0x89, b'P', b'N', b'G']),
            ClipboardContent::FileList(vec![FileEntry {
                id: 0,
                name: "a.txt".into(),
                size: 9,
                is_dir: false,
            }]),
        ];
        for c in items {
            let mut tx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
            let mut rx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
            let msgs = tx.on_local_change(c.clone());
            assert_eq!(deliver(&mut rx, msgs), Some(c));
        }
    }

    #[test]
    fn large_content_is_chunked_and_reassembled() {
        let mut tx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let mut rx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let big = ClipboardContent::Png(vec![7u8; CHUNK_SIZE * 3 + 10]);
        let msgs = tx.on_local_change(big.clone());
        assert!(matches!(msgs[0], Message::ClipboardOffer { .. }));
        assert_eq!(msgs.len(), 1 + 4);
        for m in &msgs[1..] {
            if let Message::ClipboardChunk { data, .. } = m {
                assert!(data.len() <= CHUNK_SIZE);
            }
        }
        assert_eq!(deliver(&mut rx, msgs), Some(big));
    }

    #[test]
    fn out_of_order_chunk_discards_offer() {
        let mut tx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let mut rx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let mut msgs = tx.on_local_change(ClipboardContent::Png(vec![1u8; CHUNK_SIZE * 2]));
        msgs.swap(1, 2);
        assert_eq!(deliver(&mut rx, msgs), None);
        assert!(!rx.has_pending_offer());
    }

    #[test]
    fn incomplete_offer_is_dropped_after_10s() {
        let mut tx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let mut rx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let msgs = tx.on_local_change(ClipboardContent::Png(vec![1u8; CHUNK_SIZE * 2]));
        let t0 = Instant::now();
        rx.on_message_at(msgs[0].clone(), t0);
        rx.on_message_at(msgs[1].clone(), t0 + Duration::from_secs(5));
        assert!(rx.has_pending_offer());
        // chunk final chega tarde demais: oferta já descartada
        let late = t0 + Duration::from_secs(11);
        assert_eq!(rx.on_message_at(msgs[2].clone(), late), None);
        assert!(!rx.has_pending_offer());
    }

    #[test]
    fn complete_before_timeout_is_accepted() {
        let mut tx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let mut rx = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let c = ClipboardContent::Png(vec![1u8; CHUNK_SIZE * 2]);
        let msgs = tx.on_local_change(c.clone());
        let t0 = Instant::now();
        let mut out = None;
        for (i, m) in msgs.into_iter().enumerate() {
            out = rx
                .on_message_at(m, t0 + Duration::from_secs(i as u64 * 2))
                .or(out);
        }
        assert_eq!(out, Some(c));
    }

    #[test]
    fn too_large_is_skipped() {
        let mut tx = ClipboardSync::new(ClipboardConfig {
            mode: ClipboardMode::OnCopy,
            max_bytes: 100,
            ..ClipboardConfig::default()
        });
        let msgs = tx.on_local_change(ClipboardContent::Text("x".repeat(200)));
        assert_eq!(
            msgs,
            vec![Message::ClipboardSkipped {
                reason: "too_large".into()
            }]
        );
        assert!(
            !tx.on_local_change(ClipboardContent::Text("curto".into()))
                .is_empty()
        );
    }

    #[test]
    fn applied_remote_content_is_not_echoed() {
        let mut a = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let mut b = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        let c = ClipboardContent::Text("segredo".into());
        let applied = deliver(&mut b, a.on_local_change(c.clone())).unwrap();
        // o backend de B reporta mudança local com o mesmo conteúdo
        assert!(b.on_local_change(applied).is_empty());
        // conteúdo diferente sai normalmente
        assert!(
            !b.on_local_change(ClipboardContent::Text("outro".into()))
                .is_empty()
        );
    }

    #[test]
    fn on_copy_offers_immediately_on_switch_waits() {
        let c = ClipboardContent::Text("oi".into());
        let mut copy = ClipboardSync::new(cfg(ClipboardMode::OnCopy));
        assert!(!copy.on_local_change(c.clone()).is_empty());
        assert!(copy.on_switch(Some(c.clone())).is_empty());

        let mut sw = ClipboardSync::new(cfg(ClipboardMode::OnSwitch));
        assert!(sw.on_local_change(c.clone()).is_empty());
        assert!(!sw.on_switch(Some(c.clone())).is_empty());
        // mesmo conteúdo, nova troca: não reenvia
        assert!(sw.on_switch(Some(c)).is_empty());
        assert!(sw.on_switch(None).is_empty());
    }

    #[test]
    fn send_now_forces_in_any_mode_and_repeats() {
        let c = ClipboardContent::Text("oi".into());
        let mut sw = ClipboardSync::new(cfg(ClipboardMode::OnSwitch));
        assert!(!sw.send_now(Some(c.clone())).is_empty());
        assert!(!sw.send_now(Some(c)).is_empty());
        assert!(sw.send_now(None).is_empty());
    }

    #[test]
    fn disabled_or_filtered_format_sends_nothing() {
        let mut off = ClipboardSync::new(ClipboardConfig {
            enabled: false,
            mode: ClipboardMode::OnCopy,
            ..ClipboardConfig::default()
        });
        assert!(
            off.send_now(Some(ClipboardContent::Text("x".into())))
                .is_empty()
        );

        let mut only_text = ClipboardSync::new(ClipboardConfig {
            mode: ClipboardMode::OnCopy,
            formats: vec![ClipboardFormat::Text],
            ..ClipboardConfig::default()
        });
        assert!(
            only_text
                .on_local_change(ClipboardContent::Png(vec![1]))
                .is_empty()
        );
    }

    #[test]
    fn oversized_remote_offer_is_ignored() {
        let mut rx = ClipboardSync::new(ClipboardConfig {
            max_bytes: 10,
            ..ClipboardConfig::default()
        });
        rx.on_message(Message::ClipboardOffer {
            offer_id: 1,
            formats: vec![ClipboardFormat::Png],
            total: 1000,
        });
        assert!(!rx.has_pending_offer());
    }
}

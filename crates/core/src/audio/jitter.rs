//! Buffer de jitter adaptativo, dirigido pelo relógio de reprodução (`read`).
//!
//! Os pacotes entram por `seq` num anel de [`SLOTS`] posições, o que reordena de graça. O
//! tempo de espera é o próprio atraso de reprodução (o alvo): um pacote ausente só vira perda na
//! hora de tocá-lo, e então chega tarde demais e é descartado. O alvo sobe a cada underrun
//! (esvaziou sem nada à frente) e desce devagar depois de um período estável.

use super::{AudioError, AudioPacket, CHANNELS, FRAMES_PER_PACKET, PACKET_PAYLOAD_BYTES};

/// Posições do anel (320 ms): folga sobre o teto de 80 ms mais o jitter reordenável.
const SLOTS: usize = 64;
const SAMPLES_PER_PACKET: usize = FRAMES_PER_PACKET * CHANNELS;
const FRAMES_PER_MS: usize = 48;
/// Teto do alvo, em ms.
const MAX_TARGET_MS: u32 = 80;
/// Quanto o alvo sobe a cada underrun, em ms.
const RAISE_STEP_MS: u32 = 10;
/// Pacotes tocados sem incidente (2 s) até o alvo descer 1 ms.
const STABLE_PACKETS: u32 = 400;
/// Excesso sobre o alvo (ms) a partir do qual o atraso acumulado é cortado (rajada pós-parada).
const MAX_EXCESS_MS: usize = 30;
/// Rampa de entrada (2 ms) no primeiro pacote real depois de uma perda.
const FADE_IN_FRAMES: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadStatus {
    /// Tudo veio de pacotes reais.
    Played,
    /// Houve perda preenchida (último quadro repetido com fade).
    Concealed,
    /// Reabastecendo até o alvo; a saída é silêncio.
    Buffering,
    /// Esvaziou nesta leitura; o alvo subiu e o buffer reabastece.
    Underrun,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JitterStats {
    pub target_ms: u32,
    pub level_ms: f32,
    pub received: u64,
    pub played: u64,
    pub lost: u64,
    pub late: u64,
    pub duplicates: u64,
    pub reordered: u64,
    pub underruns: u64,
    pub resyncs: u64,
    /// Pacotes descartados para recuperar atraso depois de uma rajada.
    pub skipped: u64,
}

impl JitterStats {
    /// Perda em % dos pacotes que deveriam ter tocado.
    pub fn loss_pct(&self) -> f32 {
        let total = self.played + self.lost;
        if total == 0 {
            0.0
        } else {
            self.lost as f32 * 100.0 / total as f32
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cur {
    Real,
    Concealed,
}

/// Buffer de jitter adaptativo (começa em 20 ms, teto 80 ms). Só aceita o formato do pipeline
/// (240 frames estéreo s16le por pacote).
#[derive(Debug)]
pub struct JitterBuffer {
    slots: Vec<i16>,
    present: Vec<bool>,
    has_base: bool,
    started_once: bool,
    buffering: bool,
    next_seq: u64,
    highest_seq: u64,
    /// Frames já lidos do pacote `next_seq`.
    pos: usize,
    cur: Cur,
    last_frame: [i16; SAMPLES_PER_PACKET],
    conceal: [i16; SAMPLES_PER_PACKET],
    /// Ganho com que a última saída terminou (1.0 após pacote real, 0.0 após fade completo).
    conceal_gain: f32,
    tail_pos: usize,
    arrival_level: usize,
    after_conceal: bool,
    fade_in_left: usize,
    floor_ms: u32,
    target_ms: u32,
    stable: u32,
    stats: JitterStats,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new(20)
    }
}

impl JitterBuffer {
    /// `initial_target_ms` é também o piso do alvo (limitado a 5..=80 ms).
    pub fn new(initial_target_ms: u32) -> Self {
        let t = initial_target_ms.clamp(5, MAX_TARGET_MS);
        Self {
            slots: vec![0; SLOTS * SAMPLES_PER_PACKET],
            present: vec![false; SLOTS],
            has_base: false,
            started_once: false,
            buffering: true,
            next_seq: 0,
            highest_seq: 0,
            pos: 0,
            cur: Cur::Real,
            last_frame: [0; SAMPLES_PER_PACKET],
            conceal: [0; SAMPLES_PER_PACKET],
            conceal_gain: 0.0,
            tail_pos: FRAMES_PER_PACKET,
            arrival_level: 0,
            after_conceal: false,
            fade_in_left: 0,
            floor_ms: t,
            target_ms: t,
            stable: 0,
            stats: JitterStats::default(),
        }
    }

    pub fn target_ms(&self) -> u32 {
        self.target_ms
    }

    pub fn target_frames(&self) -> usize {
        self.target_ms as usize * FRAMES_PER_MS
    }

    /// Frames enfileirados à frente do cursor de reprodução (profundidade na linha do tempo).
    pub fn level_frames(&self) -> usize {
        if !self.has_base || self.highest_seq < self.next_seq {
            return 0;
        }
        let packets = (self.highest_seq - self.next_seq + 1) as usize;
        (packets * FRAMES_PER_PACKET).saturating_sub(self.pos)
    }

    /// Nível médio estimado, a medida que o controle de deriva deve usar com `target_frames()`.
    /// O nível instantâneo oscila em dente de serra de 1 pacote (5 ms) entre chegadas; esta
    /// amostra, tirada logo depois da última chegada (sempre na mesma fase) menos meio pacote,
    /// só se move com a deriva real.
    pub fn mean_level_frames(&self) -> usize {
        self.arrival_level.saturating_sub(FRAMES_PER_PACKET / 2)
    }

    pub fn stats(&self) -> JitterStats {
        JitterStats {
            target_ms: self.target_ms,
            level_ms: self.level_frames() as f32 / FRAMES_PER_MS as f32,
            ..self.stats
        }
    }

    pub fn push(&mut self, p: &AudioPacket) -> Result<(), AudioError> {
        if usize::from(p.channels) != CHANNELS || usize::from(p.frames) != FRAMES_PER_PACKET {
            return Err(AudioError::InvalidPacket);
        }
        self.push_payload(p.seq, &p.payload)
    }

    /// Enfileira o payload PCM s16le de um pacote (sem alocar).
    pub fn push_payload(&mut self, seq: u64, payload: &[u8]) -> Result<(), AudioError> {
        if payload.len() != PACKET_PAYLOAD_BYTES {
            return Err(AudioError::InvalidPacket);
        }
        self.stats.received += 1;
        if !self.has_base {
            self.has_base = true;
            self.next_seq = seq;
            self.highest_seq = seq;
        } else if self.buffering && !self.started_once && seq < self.next_seq {
            // Reordenação antes da primeira reprodução: ainda dá para recuar a base.
            if self.highest_seq - seq < SLOTS as u64 {
                self.next_seq = seq;
            }
        }
        if seq < self.next_seq || (seq == self.next_seq && self.pos > 0) {
            self.stats.late += 1;
            return Ok(());
        }
        if seq >= self.next_seq + SLOTS as u64 {
            self.resync(seq);
        }
        let idx = (seq % SLOTS as u64) as usize;
        if self.present[idx] {
            self.stats.duplicates += 1;
            return Ok(());
        }
        let dst = &mut self.slots[idx * SAMPLES_PER_PACKET..(idx + 1) * SAMPLES_PER_PACKET];
        for (d, s) in dst.iter_mut().zip(payload.chunks_exact(2)) {
            *d = i16::from_le_bytes([s[0], s[1]]);
        }
        self.present[idx] = true;
        if seq > self.highest_seq {
            self.highest_seq = seq;
        } else {
            self.stats.reordered += 1;
        }
        self.arrival_level = self.level_frames();
        Ok(())
    }

    /// Preenche `out` (estéreo intercalado, número par de amostras) com o áudio a tocar agora.
    /// Chamado pelo relógio de reprodução; devolve o pior estado ocorrido na leitura.
    pub fn read(&mut self, out: &mut [i16]) -> ReadStatus {
        debug_assert!(out.len() % CHANNELS == 0);
        let total = out.len() / CHANNELS;
        let mut status = ReadStatus::Played;
        let mut w = 0;
        while w < total {
            if self.buffering {
                if self.tail_pos < FRAMES_PER_PACKET {
                    let n = (total - w).min(FRAMES_PER_PACKET - self.tail_pos);
                    out[w * CHANNELS..(w + n) * CHANNELS].copy_from_slice(
                        &self.conceal[self.tail_pos * CHANNELS..(self.tail_pos + n) * CHANNELS],
                    );
                    self.tail_pos += n;
                    w += n;
                    status = status.max(ReadStatus::Buffering);
                    continue;
                }
                if !self.try_start() {
                    out[w * CHANNELS..].fill(0);
                    status = status.max(ReadStatus::Buffering);
                    break;
                }
            }
            if self.pos == 0 {
                match self.begin_packet() {
                    ReadStatus::Played => {}
                    s => status = status.max(s),
                }
                if self.buffering {
                    // Underrun: o fade da perda sai pelo ramo de reabastecimento.
                    status = ReadStatus::Underrun;
                    continue;
                }
            }
            let n = (total - w).min(FRAMES_PER_PACKET - self.pos);
            let dst = &mut out[w * CHANNELS..(w + n) * CHANNELS];
            let (a, b) = (self.pos * CHANNELS, (self.pos + n) * CHANNELS);
            match self.cur {
                Cur::Concealed => dst.copy_from_slice(&self.conceal[a..b]),
                Cur::Real => {
                    let base = (self.next_seq % SLOTS as u64) as usize * SAMPLES_PER_PACKET;
                    dst.copy_from_slice(&self.slots[base + a..base + b]);
                    if self.fade_in_left > 0 {
                        for frame in dst.chunks_exact_mut(CHANNELS) {
                            if self.fade_in_left == 0 {
                                break;
                            }
                            let g =
                                (FADE_IN_FRAMES - self.fade_in_left) as f32 / FADE_IN_FRAMES as f32;
                            self.fade_in_left -= 1;
                            for s in frame {
                                *s = (f32::from(*s) * g) as i16;
                            }
                        }
                    }
                }
            }
            self.pos += n;
            w += n;
            if self.pos == FRAMES_PER_PACKET {
                self.end_packet();
            }
        }
        status
    }

    fn resync(&mut self, seq: u64) {
        self.present.fill(false);
        self.next_seq = seq;
        self.highest_seq = seq;
        self.pos = 0;
        self.buffering = true;
        self.started_once = false;
        self.after_conceal = true;
        self.stats.resyncs += 1;
    }

    /// Sai do reabastecimento quando há profundidade suficiente; salta a lacuna inicial.
    fn try_start(&mut self) -> bool {
        if !self.has_base || self.level_frames() < self.target_frames() {
            return false;
        }
        while self.next_seq < self.highest_seq
            && !self.present[(self.next_seq % SLOTS as u64) as usize]
        {
            self.next_seq += 1;
        }
        self.pos = 0;
        self.buffering = false;
        self.started_once = true;
        true
    }

    /// Decide a origem do pacote `next_seq`. Devolve `Played`, `Concealed` ou `Underrun`.
    fn begin_packet(&mut self) -> ReadStatus {
        if self.level_frames() > self.target_frames() + MAX_EXCESS_MS * FRAMES_PER_MS {
            self.skip_excess();
        }
        let idx = (self.next_seq % SLOTS as u64) as usize;
        if self.present[idx] {
            self.cur = Cur::Real;
            self.last_frame.copy_from_slice(
                &self.slots[idx * SAMPLES_PER_PACKET..(idx + 1) * SAMPLES_PER_PACKET],
            );
            if self.after_conceal {
                self.fade_in_left = FADE_IN_FRAMES;
                self.after_conceal = false;
            }
            ReadStatus::Played
        } else if self.highest_seq > self.next_seq {
            self.cur = Cur::Concealed;
            self.gen_conceal();
            self.stats.lost += 1;
            self.stable = 0;
            self.after_conceal = true;
            ReadStatus::Concealed
        } else {
            self.gen_conceal();
            self.tail_pos = 0;
            self.buffering = true;
            self.after_conceal = true;
            self.stable = 0;
            self.stats.underruns += 1;
            self.target_ms = (self.target_ms + RAISE_STEP_MS).min(MAX_TARGET_MS);
            ReadStatus::Underrun
        }
    }

    /// Descarta os pacotes mais antigos até sobrar o alvo mais um pacote de folga.
    fn skip_excess(&mut self) {
        while self.next_seq < self.highest_seq
            && self.level_frames() > self.target_frames() + FRAMES_PER_PACKET
        {
            self.present[(self.next_seq % SLOTS as u64) as usize] = false;
            self.next_seq += 1;
            self.stats.skipped += 1;
        }
        self.after_conceal = true;
        self.stable = 0;
    }

    fn end_packet(&mut self) {
        if self.cur == Cur::Real {
            self.present[(self.next_seq % SLOTS as u64) as usize] = false;
            self.conceal_gain = 1.0;
            self.stats.played += 1;
            self.stable += 1;
            if self.stable >= STABLE_PACKETS {
                self.stable = 0;
                self.target_ms = self.target_ms.saturating_sub(1).max(self.floor_ms);
            }
        }
        self.next_seq += 1;
        self.pos = 0;
    }

    /// Último quadro repetido com fade linear do ganho atual até zero.
    fn gen_conceal(&mut self) {
        let g0 = self.conceal_gain;
        for (f, (d, s)) in self
            .conceal
            .chunks_exact_mut(CHANNELS)
            .zip(self.last_frame.chunks_exact(CHANNELS))
            .enumerate()
        {
            let g = g0 * (1.0 - f as f32 / FRAMES_PER_PACKET as f32);
            for (d, s) in d.iter_mut().zip(s) {
                *d = (f32::from(*s) * g).round() as i16;
            }
        }
        self.conceal_gain = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPP: usize = FRAMES_PER_PACKET;

    /// Sample (ambos os canais) = índice global do frame: rampa com passo 1.
    fn payload(seq: u64) -> Vec<u8> {
        let mut v = Vec::with_capacity(PACKET_PAYLOAD_BYTES);
        for f in 0..FPP as u64 {
            let s = ((seq * FPP as u64 + f) % 30_000) as i16;
            for _ in 0..CHANNELS {
                v.extend_from_slice(&s.to_le_bytes());
            }
        }
        v
    }

    /// Passo da rampa de teste (dá a volta em 30000).
    fn step_ok(a: i16, b: i16) -> bool {
        b == a + 1 || (a == 29_999 && b == 0)
    }

    struct Run {
        jb: JitterBuffer,
        out: Vec<i16>,
        statuses: Vec<ReadStatus>,
        max_target: u32,
    }

    /// `arrivals`: (instante de chegada em µs, seq). Reprodução a cada 5 ms (deslocada 0,5 ms).
    fn simulate(mut arrivals: Vec<(u64, u64)>, secs: u64, floor_ms: u32) -> Run {
        arrivals.sort();
        let mut jb = JitterBuffer::new(floor_ms);
        let mut out = Vec::new();
        let mut statuses = Vec::new();
        let mut max_target = 0;
        let mut a = 0;
        let mut buf = [0i16; SAMPLES_PER_PACKET];
        let ticks = secs * 200;
        for k in 0..ticks {
            let t = k * 5000 + 500;
            while a < arrivals.len() && arrivals[a].0 <= t {
                jb.push_payload(arrivals[a].1, &payload(arrivals[a].1))
                    .unwrap();
                a += 1;
            }
            statuses.push(jb.read(&mut buf));
            max_target = max_target.max(jb.target_ms());
            out.extend_from_slice(&buf);
        }
        Run {
            jb,
            out,
            statuses,
            max_target,
        }
    }

    fn steady(n: u64, delay_us: impl Fn(u64) -> u64) -> Vec<(u64, u64)> {
        (0..n).map(|i| (i * 5000 + delay_us(i), i)).collect()
    }

    fn started(r: &Run) -> usize {
        r.statuses
            .iter()
            .position(|s| *s == ReadStatus::Played)
            .unwrap()
    }

    #[test]
    fn steady_stream_is_continuous_and_exact() {
        let r = simulate(steady(400, |_| 1000), 2, 20);
        let s = started(&r);
        assert!(s <= 5);
        assert!(r.statuses[s..].iter().all(|x| *x == ReadStatus::Played));
        let o = &r.out[s * SAMPLES_PER_PACKET..];
        for w in o.chunks_exact(2).collect::<Vec<_>>().windows(2) {
            assert!(step_ok(w[0][0], w[1][0]));
            assert_eq!(w[0][0], w[0][1]);
        }
        assert_eq!(r.jb.target_ms(), 20);
    }

    #[test]
    fn reorders_within_window() {
        // Cada par (2k, 2k+1) chega invertido.
        let arr = steady(400, |i| if i % 2 == 0 { 6000 } else { 0 });
        let r = simulate(arr, 2, 20);
        let s = started(&r);
        assert!(r.statuses[s..].iter().all(|x| *x == ReadStatus::Played));
        let o = &r.out[s * SAMPLES_PER_PACKET..];
        for w in o.chunks_exact(2).collect::<Vec<_>>().windows(2) {
            assert!(step_ok(w[0][0], w[1][0]));
        }
        let st = r.jb.stats();
        assert!(st.reordered > 100);
        assert_eq!(st.late, 0);
        assert_eq!(st.lost, 0);
    }

    #[test]
    fn late_packet_is_dropped_and_gap_concealed() {
        let arr = steady(400, |i| if i == 100 { 100_000 } else { 1000 });
        let r = simulate(arr, 2, 20);
        let st = r.jb.stats();
        assert_eq!(st.late, 1);
        assert_eq!(st.lost, 1);
        assert_eq!(st.underruns, 0);
        assert_eq!(
            r.statuses
                .iter()
                .filter(|s| **s == ReadStatus::Concealed)
                .count(),
            1
        );
        // Depois da perda a linha do tempo segue alinhada: o bloco seguinte é o pacote 101.
        let k = r
            .statuses
            .iter()
            .position(|s| *s == ReadStatus::Concealed)
            .unwrap();
        let o: Vec<i16> = r.out.chunks_exact(2).map(|f| f[0]).collect();
        assert_eq!(o[(k + 1) * FPP + 130] as usize, (101 * FPP + 130) % 30_000);
    }

    #[test]
    fn loss_repeats_last_frame_with_fade() {
        let arr: Vec<_> = steady(400, |_| 1000)
            .into_iter()
            .filter(|(_, s)| *s != 100)
            .collect();
        let r = simulate(arr, 2, 20);
        let first = (0..r.statuses.len())
            .find(|&k| r.statuses[k] == ReadStatus::Concealed)
            .unwrap();
        let blk = |k: usize| &r.out[k * SAMPLES_PER_PACKET..(k + 1) * SAMPLES_PER_PACKET];
        let (prev, lost, next) = (blk(first - 1), blk(first), blk(first + 1));
        for f in 0..FPP {
            let g = 1.0 - f as f32 / FPP as f32;
            let want = (f32::from(prev[f * 2]) * g).round() as i16;
            assert_eq!(lost[f * 2], want);
        }
        // Sem salto brusco na volta: o pacote seguinte entra com rampa de 2 ms a partir de 0.
        assert_eq!(next[0], 0);
        assert!(next[FADE_IN_FRAMES * 2 + 2] != 0);
        assert_eq!(next[(FPP - 1) * 2] as usize, (101 * FPP + FPP - 1) % 30_000);
    }

    #[test]
    fn target_rises_on_underrun_and_decays_when_stable() {
        // Estável, uma pausa de 60 ms na rede (rajada) no segundo 2, estável de novo.
        let arr = steady(8000, |i| {
            if (400..412).contains(&i) {
                60_000 + (412 - i) * 5000
            } else {
                1000
            }
        });
        let r = simulate(arr, 40, 20);
        let st = r.jb.stats();
        assert!(st.underruns >= 1);
        assert!(st.underruns <= 3);
        assert!(r.max_target >= 30, "alvo não subiu: {}", r.max_target);
        // ...e volta ao piso depois de ~20 s estáveis, sem descer abaixo dele.
        assert_eq!(st.target_ms, 20);
        assert!(st.target_ms <= MAX_TARGET_MS);
    }

    #[test]
    fn target_is_capped_at_80ms() {
        // Paradas repetidas de 200 ms.
        // A cada 0,5 s a rede segura 40 pacotes (200 ms) e solta tudo junto.
        let arr = steady(4000, |i| {
            if (50..90).contains(&(i % 100)) {
                (90 - i % 100) * 5000 + 1000
            } else {
                1000
            }
        });
        let r = simulate(arr, 20, 20);
        assert_eq!(r.max_target, MAX_TARGET_MS);
        assert!(r.jb.stats().skipped > 0);
    }

    #[test]
    fn far_ahead_sequence_resyncs() {
        let mut jb = JitterBuffer::new(20);
        for s in 0..8 {
            jb.push_payload(s, &payload(s)).unwrap();
        }
        let mut buf = [0i16; SAMPLES_PER_PACKET];
        assert_eq!(jb.read(&mut buf), ReadStatus::Played);
        jb.push_payload(10_000, &payload(10_000)).unwrap();
        assert_eq!(jb.stats().resyncs, 1);
        assert_eq!(jb.read(&mut buf), ReadStatus::Buffering);
    }

    #[test]
    fn rejects_bad_payload_and_format() {
        let mut jb = JitterBuffer::default();
        assert!(jb.push_payload(0, &[0; 10]).is_err());
        let p = AudioPacket {
            stream: 0,
            seq: 0,
            ts_samples: 0,
            codec: 0,
            channels: 1,
            frames: 240,
            payload: vec![0; 480],
        };
        assert!(jb.push(&p).is_err());
    }
}

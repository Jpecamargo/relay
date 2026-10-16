//! dono: C1
use super::*;
use crate::types::{InputEvent, Role};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

/// Conta alocações só da thread que ligou `COUNTING`, para não depender de outros testes.
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                ALLOCS.with(|a| a.set(a.get() + 1));
                BYTES.with(|b| b.set(b.get() + l.size()));
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                ALLOCS.with(|a| a.set(a.get() + 1));
                BYTES.with(|b| b.set(b.get() + n));
            }
        });
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn measure<R>(f: impl FnOnce() -> R) -> (R, usize, usize) {
    ALLOCS.with(|a| a.set(0));
    BYTES.with(|b| b.set(0));
    COUNTING.with(|c| c.set(true));
    let r = f();
    COUNTING.with(|c| c.set(false));
    (r, ALLOCS.with(Cell::get), BYTES.with(Cell::get))
}

fn mv(seq: u64) -> FastPacket {
    FastPacket {
        seq,
        ts_us: seq * 7,
        body: FastBody::MouseMove {
            dx: -3,
            dy: i16::MAX,
        },
    }
}

fn scroll(seq: u64) -> FastPacket {
    FastPacket {
        seq,
        ts_us: u64::MAX,
        body: FastBody::Scroll {
            dx: i16::MIN,
            dy: 120,
            phase: ScrollPhase::Momentum,
        },
    }
}

#[test]
fn layout_exato() {
    let mut buf = [0u8; FAST_PACKET_MAX];
    let n = mv(0x0102).encode(&mut buf).unwrap();
    assert_eq!(n, 21);
    assert_eq!(buf[0], 1);
    assert_eq!(&buf[1..9], &0x0102u64.to_le_bytes());
    assert_eq!(&buf[9..17], &(0x0102u64 * 7).to_le_bytes());
    assert_eq!(&buf[17..19], &(-3i16).to_le_bytes());
    assert_eq!(&buf[19..21], &i16::MAX.to_le_bytes());

    let n = scroll(1).encode(&mut buf).unwrap();
    assert_eq!(n, 22);
    assert_eq!(buf[0], 2);
    assert_eq!(buf[21], ScrollPhase::Momentum.as_u8());
    assert_eq!((FAST_HEADER_LEN, MOUSE_MOVE_LEN, SCROLL_LEN), (17, 21, 22));
}

#[test]
fn roundtrip_e_zero_alocacoes() {
    let (_, allocs, _) = measure(|| {
        let mut buf = [0u8; FAST_PACKET_MAX];
        for i in 0..10_000u64 {
            for p in [mv(i), scroll(i)] {
                let n = p.encode(&mut buf).unwrap();
                assert_eq!(FastPacket::decode(&buf[..n]).unwrap(), p);
            }
        }
    });
    assert_eq!(allocs, 0);
}

#[test]
fn decode_nunca_entra_em_panic() {
    let mut buf = [0u8; FAST_PACKET_MAX];
    let n = scroll(9).encode(&mut buf).unwrap();
    for len in 0..n {
        assert!(FastPacket::decode(&buf[..len]).is_err(), "truncado {len}");
    }
    // Todos os comprimentos de 0 a 64 para cada kind (válido, desconhecido e fase inválida).
    let mut data = [0xAAu8; 64];
    for kind in [0u8, 1, 2, 3, 255] {
        data[0] = kind;
        for len in 0..=64 {
            let _ = FastPacket::decode(&data[..len]);
        }
    }
    data[0] = 2;
    data[21] = 99;
    assert!(matches!(
        FastPacket::decode(&data[..22]),
        Err(ProtocolError::BadPhase(99))
    ));
    data[0] = 9;
    assert!(matches!(
        FastPacket::decode(&data[..22]),
        Err(ProtocolError::UnknownKind(9))
    ));
    // xorshift: bytes aleatórios.
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    for _ in 0..10_000 {
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let len = (next() % 65) as usize;
        let mut b = [0u8; 64];
        b.iter_mut().for_each(|x| *x = next() as u8);
        let _ = FastPacket::decode(&b[..len]);
    }
}

#[test]
fn seq_filter() {
    let mut f = SeqFilter::default();
    assert!(f.accept(0));
    assert!(!f.accept(0));
    assert!(f.accept(5));
    assert!(!f.accept(3));
    assert!(!f.accept(5));
    assert!(f.accept(1_000));
    assert!(f.accept(u64::MAX));
    assert!(!f.accept(u64::MAX));
}

fn msgs() -> Vec<Message> {
    vec![
        Message::HelloAck,
        Message::Hello {
            protocol_version: 1,
            device_id: "mac".into(),
            role: Role::Dialer,
            app_version: "0.1".into(),
        },
        Message::Ping { id: 7, ts_us: 99 },
        Message::Input {
            seq: 3,
            ts_us: 4,
            event: InputEvent::MouseMove { dx: 1, dy: -1 },
        },
        Message::Bye,
    ]
}

#[test]
fn frames_byte_a_byte() {
    let mut wire = Vec::new();
    for m in msgs() {
        FrameWriter.encode(&m, &mut wire).unwrap();
    }
    assert_eq!(
        u32::from_le_bytes(wire[..4].try_into().unwrap()) as usize + 4,
        {
            let mut one = Vec::new();
            FrameWriter.encode(&msgs()[0], &mut one).unwrap();
            one.len()
        }
    );
    let mut r = FrameReader::default();
    let mut got = Vec::new();
    for b in &wire {
        got.extend(r.push(std::slice::from_ref(b)).unwrap());
    }
    assert_eq!(got, msgs());
    // Tudo de uma vez e em pedaços desalinhados.
    assert_eq!(FrameReader::default().push(&wire).unwrap(), msgs());
    let mut r = FrameReader::default();
    let mut got = Vec::new();
    for c in wire.chunks(5) {
        got.extend(r.push(c).unwrap());
    }
    assert_eq!(got, msgs());
}

#[test]
fn frame_grande_demais_e_rejeitado_sem_alocar() {
    let hdr = ((MAX_FRAME + 1) as u32).to_le_bytes();
    let (res, _, bytes) = measure(|| {
        let mut r = FrameReader::default();
        let res = r.push(&hdr);
        (res, r)
    });
    assert!(matches!(
        res.0,
        Err(ProtocolError::FrameTooLarge(n)) if n == MAX_FRAME + 1
    ));
    assert!(bytes < 1024, "alocou {bytes} bytes");
    // Declarado absurdo (4 GiB - 1), chegando fatiado.
    let mut r = FrameReader::default();
    assert!(r.push(&hdr[..2]).unwrap().is_empty());
    assert!(matches!(
        r.push(&[0xFF, 0xFF]),
        Err(ProtocolError::FrameTooLarge(_))
    ));
}

#[test]
fn frame_lixo_vira_erro() {
    let mut wire = 3u32.to_le_bytes().to_vec();
    wire.extend_from_slice(&[250, 250, 250]);
    assert!(FrameReader::default().push(&wire).is_err());
    assert!(FrameReader::default().push(&0u32.to_le_bytes()).is_err());
}

#[test]
fn writer_rejeita_mensagem_gigante() {
    let m = Message::ForegroundApp {
        exe: "x".repeat(MAX_FRAME + 10),
    };
    let mut out = vec![1, 2, 3];
    assert!(FrameWriter.encode(&m, &mut out).is_err());
    assert_eq!(out, [1, 2, 3]);
}

use std::path::Path;
use std::sync::atomic::AtomicBool;

use blake2::{Blake2s256, Digest};
use tokio::io::duplex;

use super::*;
use crate::crypto::{MemoryKeyStore, device_id_of, load_or_create_identity};

fn ident() -> StaticKeypair {
    load_or_create_identity(&MemoryKeyStore::default()).unwrap()
}

fn peer_of(k: &StaticKeypair, name: &str) -> PairedPeer {
    PairedPeer {
        device_id: device_id_of(&k.public),
        name: name.into(),
        public_key: k.public,
        mac_address: [0; 6],
    }
}

fn hash_file(p: &Path) -> [u8; 32] {
    Blake2s256::digest(std::fs::read(p).unwrap()).into()
}

/// Hash por caminho relativo de toda a árvore.
fn tree(root: &Path) -> Vec<(String, Option<[u8; 32]>)> {
    fn go(base: &Path, dir: &Path, out: &mut Vec<(String, Option<[u8; 32]>)>) {
        let mut es: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        es.sort_by_key(|e| e.file_name());
        for e in es {
            let rel = e
                .path()
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if e.path().is_dir() {
                out.push((rel, None));
                go(base, &e.path(), out);
            } else {
                out.push((rel, Some(hash_file(&e.path()))));
            }
        }
    }
    let mut out = Vec::new();
    go(root, root, &mut out);
    out
}

struct Pair {
    server: TransferServer,
    client: TransferClient,
}

fn pair(paths: Vec<PathBuf>) -> Pair {
    let (mac, pc) = (ident(), ident());
    let (mac_p, pc_p) = (peer_of(&mac, "Mac"), peer_of(&pc, "PC"));
    Pair {
        server: TransferServer::new(pc, vec![mac_p], paths),
        client: TransferClient::new(mac, pc_p),
    }
}

async fn run_download(
    p: &Pair,
    ids: &[u32],
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(TransferProgress),
) -> Result<Vec<PathBuf>, TransferError> {
    let (mut a, mut b) = duplex(1 << 17);
    let (c, _s) = tokio::join!(
        async {
            let r = p
                .client
                .download(&mut a, ids, dest, 7, cancel, progress)
                .await;
            drop(a); // fecha a conexão, como o TCP faria
            r
        },
        async {
            let r = p.server.serve_connection(&mut b).await;
            drop(b);
            r
        }
    );
    c
}

fn make_big(path: &Path, len: usize) {
    let data: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(31) >> 3) as u8).collect();
    std::fs::write(path, data).unwrap();
}

#[tokio::test]
async fn file_and_nested_dir_arrive_identical_and_progress_is_throttled() {
    let src = tempfile::tempdir().unwrap();
    let big = src.path().join("big.bin");
    make_big(&big, 20 * 1024 * 1024);
    let dir = src.path().join("pasta");
    std::fs::create_dir_all(dir.join("a/b")).unwrap();
    std::fs::create_dir_all(dir.join("vazia")).unwrap();
    std::fs::write(dir.join("raiz.txt"), b"raiz").unwrap();
    std::fs::write(dir.join("a/b/fundo.txt"), b"fundo").unwrap();
    std::fs::write(dir.join("a/zero.bin"), b"").unwrap();

    let dest = tempfile::tempdir().unwrap();
    let p = pair(vec![big.clone(), dir.clone()]);
    let mut events = Vec::new();
    let t0 = Instant::now();
    let out = run_download(
        &p,
        &[0, 1],
        dest.path(),
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .await
    .unwrap();
    let elapsed = t0.elapsed();

    assert_eq!(out.len(), 2);
    assert_eq!(hash_file(&dest.path().join("big.bin")), hash_file(&big));
    assert_eq!(tree(&dest.path().join("pasta")), tree(&dir));
    // nada além dos dois itens (staging removido)
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 2);

    let last = events.last().unwrap();
    assert!(last.done && last.error.is_none());
    assert_eq!(last.bytes, last.total);
    assert_eq!(last.total, 20 * 1024 * 1024 + 4 + 5);
    assert!(events.iter().all(|e| e.transfer_id == 7));
    let live = events.iter().filter(|e| !e.done).count();
    let max = (elapsed.as_millis() / 100) as usize + 1;
    assert!(live <= max, "{live} eventos em {elapsed:?}");
    assert!(events.windows(2).all(|w| w[0].bytes <= w[1].bytes));
}

#[tokio::test]
async fn works_over_real_tcp() {
    let src = tempfile::tempdir().unwrap();
    let f = src.path().join("x.txt");
    std::fs::write(&f, b"tcp").unwrap();
    let dest = tempfile::tempdir().unwrap();
    let p = pair(vec![f]);
    let listener = TransferServer::bind(Some(([127, 0, 0, 1], 0).into()))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Arc::new(p.server);
    tokio::spawn(server.serve(listener));
    let mut s = TransferClient::connect(addr).await.unwrap();
    p.client
        .download(
            &mut s,
            &[0],
            dest.path(),
            1,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(dest.path().join("x.txt")).unwrap(), b"tcp");
}

#[tokio::test]
async fn cancel_midway_leaves_nothing_in_destination() {
    let src = tempfile::tempdir().unwrap();
    let big = src.path().join("big.bin");
    make_big(&big, 8 * 1024 * 1024);
    let dest = tempfile::tempdir().unwrap();
    let p = pair(vec![big]);
    let cancel = AtomicBool::new(false);
    let res = run_download(&p, &[0], dest.path(), &cancel, &mut |e| {
        if e.bytes > 1024 * 1024 {
            cancel.store(true, Ordering::Relaxed);
        }
    })
    .await;
    assert!(matches!(res, Err(TransferError::Cancelled)), "{res:?}");
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[test]
fn sanitize_blocks_traversal_and_absolute() {
    assert_eq!(
        sanitize_path("../../etc/passwd"),
        ["_", "_", "etc", "passwd"]
    );
    assert_eq!(sanitize_path("/abs/olu/to"), ["abs", "olu", "to"]);
    assert_eq!(sanitize_path("a\\..\\b"), ["a", "_", "b"]);
    assert_eq!(sanitize_path("./x//y/."), ["x", "y"]);
    assert_eq!(sanitize_component("C:"), "C_");
    assert_eq!(sanitize_component(".."), "_");
    assert_eq!(sanitize_component("a/b"), "a_b");
}

#[tokio::test]
async fn hostile_server_cannot_write_outside_destination() {
    // Servidor malicioso: fala o protocolo mas manda caminhos de traversal.
    let (mac, pc) = (ident(), ident());
    let (mac_p, pc_p) = (peer_of(&mac, "Mac"), peer_of(&pc, "PC"));
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("dest");
    std::fs::create_dir(&dest).unwrap();
    let client = TransferClient::new(mac, pc_p);
    let (nc, mut np) = (AtomicBool::new(false), |_: TransferProgress| {});
    let (mut a, mut b) = duplex(1 << 17);
    let evil = async move {
        let mut s = noise_accept(&mut b, &pc, &[mac_p]).await.unwrap();
        let (mut plain, mut c) = (Vec::new(), Vec::new());
        recv(&mut b, &mut s, &mut plain).await.unwrap();
        let mut begin = vec![MSG_BEGIN];
        begin.extend_from_slice(&3u64.to_be_bytes());
        send(&mut b, &mut s, &begin, &mut c).await.unwrap();
        let name = b"../../fugiu.txt";
        let mut e = vec![MSG_ENTRY, 0];
        e.extend_from_slice(&3u64.to_be_bytes());
        e.extend_from_slice(&(name.len() as u16).to_be_bytes());
        e.extend_from_slice(name);
        send(&mut b, &mut s, &e, &mut c).await.unwrap();
        send(&mut b, &mut s, &[&[MSG_CHUNK][..], b"abc"].concat(), &mut c)
            .await
            .unwrap();
        let mut end = vec![MSG_FILE_END];
        end.extend_from_slice(&Blake2s256::digest(b"abc"));
        send(&mut b, &mut s, &end, &mut c).await.unwrap();
        send(&mut b, &mut s, &[MSG_DONE], &mut c).await.unwrap();
    };
    let (res, ()) = tokio::join!(client.download(&mut a, &[0], &dest, 1, &nc, &mut np), evil);
    let out = res.unwrap();
    assert!(out[0].starts_with(&dest), "{out:?}");
    assert!(!root.path().join("fugiu.txt").exists());
    assert!(!root.path().parent().unwrap().join("fugiu.txt").exists());
}

#[tokio::test]
async fn corrupted_hash_is_rejected_and_cleaned() {
    let (mac, pc) = (ident(), ident());
    let (mac_p, pc_p) = (peer_of(&mac, "Mac"), peer_of(&pc, "PC"));
    let dest = tempfile::tempdir().unwrap();
    let client = TransferClient::new(mac, pc_p);
    let (nc, mut np) = (AtomicBool::new(false), |_: TransferProgress| {});
    let (mut a, mut b) = duplex(1 << 17);
    let evil = async move {
        let mut s = noise_accept(&mut b, &pc, &[mac_p]).await.unwrap();
        let (mut plain, mut c) = (Vec::new(), Vec::new());
        recv(&mut b, &mut s, &mut plain).await.unwrap();
        let mut begin = vec![MSG_BEGIN];
        begin.extend_from_slice(&3u64.to_be_bytes());
        send(&mut b, &mut s, &begin, &mut c).await.unwrap();
        let mut e = vec![MSG_ENTRY, 0];
        e.extend_from_slice(&3u64.to_be_bytes());
        e.extend_from_slice(&1u16.to_be_bytes());
        e.push(b'f');
        send(&mut b, &mut s, &e, &mut c).await.unwrap();
        send(&mut b, &mut s, &[&[MSG_CHUNK][..], b"abc"].concat(), &mut c)
            .await
            .unwrap();
        let mut end = vec![MSG_FILE_END];
        end.extend_from_slice(&[0u8; 32]);
        send(&mut b, &mut s, &end, &mut c).await.unwrap();
    };
    let (res, ()) = tokio::join!(
        client.download(&mut a, &[0], dest.path(), 1, &nc, &mut np),
        evil
    );
    assert!(matches!(res, Err(TransferError::Protocol(_))), "{res:?}");
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn unpaired_peer_is_refused() {
    let src = tempfile::tempdir().unwrap();
    let f = src.path().join("x.txt");
    std::fs::write(&f, b"segredo").unwrap();
    let (pc, intruder, mac) = (ident(), ident(), ident());
    let server = TransferServer::new(pc.clone(), vec![peer_of(&mac, "Mac")], vec![f]);
    let client = TransferClient::new(intruder, peer_of(&pc, "PC"));
    let dest = tempfile::tempdir().unwrap();
    let (nc, mut np) = (AtomicBool::new(false), |_: TransferProgress| {});
    let (mut a, mut b) = duplex(1 << 17);
    let (c, s) = tokio::join!(
        async {
            let r = client
                .download(&mut a, &[0], dest.path(), 1, &nc, &mut np)
                .await;
            drop(a);
            r
        },
        async {
            let r = server.serve_connection(&mut b).await;
            drop(b);
            r
        }
    );
    assert!(matches!(s, Err(TransferError::UnknownPeer)), "{s:?}");
    assert!(c.is_err());
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn unknown_id_reports_error_to_client() {
    let p = pair(vec![]);
    let dest = tempfile::tempdir().unwrap();
    let res = run_download(&p, &[3], dest.path(), &AtomicBool::new(false), &mut |_| {}).await;
    assert!(matches!(res, Err(TransferError::Protocol(_))), "{res:?}");
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn existing_names_are_not_overwritten() {
    let src = tempfile::tempdir().unwrap();
    let f = src.path().join("a.txt");
    std::fs::write(&f, b"novo").unwrap();
    let dest = tempfile::tempdir().unwrap();
    std::fs::write(dest.path().join("a.txt"), b"velho").unwrap();
    let p = pair(vec![f]);
    let out = run_download(&p, &[0], dest.path(), &AtomicBool::new(false), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"velho");
    assert_eq!(std::fs::read(&out[0]).unwrap(), b"novo");
    assert_eq!(out[0].file_name().unwrap(), "a (2).txt");
}

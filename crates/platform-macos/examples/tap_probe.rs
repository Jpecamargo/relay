//! dono: M1
//! Sonda manual do CGEventTap: imprime cada evento capturado. Exige a permissão de Monitoramento
//! de Entrada para o terminal (Ajustes > Privacidade e Segurança > Monitoramento de Entrada).
//!
//! Uso: `cargo run -p relay-platform-macos --example tap_probe [-- swallow]`
//! Com `swallow`, o Mac para de receber teclado/mouse (Ctrl+C no terminal não funciona se o
//! teclado for engolido: a sonda desliga o swallow sozinha após 10 s).
//!
//! Roteiro de verificação manual: mover o mouse, clicar (esq/dir/meio), rolar com o trackpad
//! (fases Began/Changed/Ended/Momentum) e com a roda, digitar (autorepeat deve vir `true` ao
//! segurar), apertar modificadores, teclas de volume/play (Media) e dar pinça/rotação no
//! trackpad. Para a reativação: deixe a sonda parada com o sistema sob carga ou dispare Secure
//! Input (campo de senha) e observe que `reactivations` só sobe, sem a captura morrer.

#[cfg(target_os = "macos")]
fn main() {
    use relay_core::platform::InputCapture;
    use relay_platform_macos::capture::MacCapture;
    use std::time::{Duration, Instant};

    let swallow = std::env::args().any(|a| a == "swallow");
    let (tx, mut rx) = rtrb::RingBuffer::new(4096);
    let mut cap = MacCapture::new();
    if let Err(e) = cap.start(tx) {
        eprintln!("falha ao iniciar a captura: {e} (conceda Monitoramento de Entrada ao terminal)");
        std::process::exit(1);
    }
    cap.set_swallow(swallow);
    println!("capturando (swallow={swallow}); Ctrl+C para sair");
    let started = Instant::now();
    loop {
        while let Ok(ev) = rx.pop() {
            println!("{ev:?}");
        }
        if swallow && cap.is_swallowing() && started.elapsed() > Duration::from_secs(10) {
            cap.set_swallow(false);
            println!("swallow desligado (limite de 10 s)");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("tap_probe só existe no macOS");
}

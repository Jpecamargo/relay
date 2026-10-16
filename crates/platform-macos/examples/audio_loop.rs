//! dono: M4
//! Teste manual de áudio: captura o microfone e toca na saída (loopback), trocando de dispositivo
//! em execução. Use fones para não gerar microfonia.
//!
//!   cargo run -p relay-platform-macos --example audio_loop            # lista e usa os padrões
//!   cargo run -p relay-platform-macos --example audio_loop -- <id-entrada|-> <id-saída|->
//!
//! Durante a execução: `i <n>` / `o <n>` trocam entrada/saída pelo índice da lista, `v <0..100>`
//! muda o volume, `q` sai. Exige permissão de Microfone para o terminal.

#[cfg(target_os = "macos")]
fn main() {
    use std::io::BufRead;

    use relay_core::platform::{
        AUDIO_CHANNELS, AUDIO_FRAME_SAMPLES, AudioCapture, AudioFrame, AudioPlayback,
    };
    use relay_platform_macos::audio::{MacAudioCapture, MacAudioPlayback, list_devices};

    let devices = list_devices().expect("listar dispositivos");
    println!("Entradas:");
    for (i, d) in devices.inputs.iter().enumerate() {
        println!(
            "  i {i}: {}{} [{}]",
            d.name,
            if d.is_default { " (padrão)" } else { "" },
            d.id
        );
    }
    println!("Saídas:");
    for (i, d) in devices.outputs.iter().enumerate() {
        println!(
            "  o {i}: {}{} [{}]",
            d.name,
            if d.is_default { " (padrão)" } else { "" },
            d.id
        );
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let pick = |i: usize| args.get(i).filter(|s| s.as_str() != "-").cloned();

    let (frame_tx, mut frame_rx) = rtrb::RingBuffer::<AudioFrame>::new(64);
    let (mut pcm_tx, pcm_rx) = rtrb::RingBuffer::<f32>::new(48_000);

    let mut capture = MacAudioCapture::default();
    capture.set_device(pick(0).as_deref()).expect("entrada");
    capture.start(frame_tx).expect("iniciar captura");

    let mut playback = MacAudioPlayback::default();
    playback.set_device(pick(1).as_deref()).expect("saída");
    playback.start(pcm_rx).expect("iniciar reprodução");

    // Ponte i16 -> f32 (no app real é o caminho de rede + jitter buffer).
    std::thread::spawn(move || {
        loop {
            while let Ok(frame) = frame_rx.pop() {
                if pcm_tx.slots() < AUDIO_FRAME_SAMPLES * AUDIO_CHANNELS {
                    continue;
                }
                for s in frame {
                    let _ = pcm_tx.push(f32::from(s) / 32768.0);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    });

    println!("Loopback ativo. Comandos: i <n>, o <n>, v <0..100>, q");
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let mut parts = line.split_whitespace();
        match (
            parts.next(),
            parts.next().and_then(|n| n.parse::<usize>().ok()),
        ) {
            (Some("q"), _) => break,
            (Some("i"), Some(n)) => match devices.inputs.get(n) {
                Some(d) => println!("{:?}", capture.set_device(Some(&d.id))),
                None => println!("índice inválido"),
            },
            (Some("o"), Some(n)) => match devices.outputs.get(n) {
                Some(d) => println!("{:?}", playback.set_device(Some(&d.id))),
                None => println!("índice inválido"),
            },
            (Some("v"), Some(n)) => playback.set_volume(n as f32 / 100.0),
            _ => println!("comando desconhecido"),
        }
    }
    capture.stop();
    playback.stop();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("audio_loop só roda no macOS");
}

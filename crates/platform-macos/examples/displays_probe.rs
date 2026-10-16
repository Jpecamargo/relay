//! dono: M3
//! Sonda manual: lista as telas e imprime a cada mudança de configuração (Ctrl+C para sair).
//! `cargo run -p relay-platform-macos --example displays_probe`
use relay_core::platform::DisplaySource;
use relay_platform_macos::displays::MacDisplays;

fn dump(d: &MacDisplays) {
    match d.list() {
        Ok(list) => list.iter().for_each(|i| println!("{i:#?}")),
        Err(e) => eprintln!("erro: {e}"),
    }
}

fn main() {
    let mut displays = MacDisplays::default();
    dump(&displays);
    displays.set_change_callback(Box::new(|| {
        println!("--- configuração de telas mudou ---");
        dump(&MacDisplays::default());
    }));
    println!("aguardando mudanças (conecte/desconecte uma tela)...");
    core_foundation::runloop::CFRunLoop::run_current();
}

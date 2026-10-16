//! relay-service: no Windows roda o serviço (item W7); nas demais plataformas só imprime o uso.

const USAGE: &str = "uso: relay-service [install|uninstall|run]\n(stub: implementação real no item W7, apenas em Windows)";

#[cfg(windows)]
fn main() {
    // O main real (dispatcher do SCM) entra aqui no W7, atrás de cfg(windows).
    println!("{USAGE}");
}

#[cfg(not(windows))]
fn main() {
    println!("{USAGE}");
}

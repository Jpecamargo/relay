//! dono: M2
//! Sonda manual do cursor: esconde, desassocia, faz warp e restaura.
//! `cargo run -p relay-platform-macos --example cursor_probe`
use std::{thread::sleep, time::Duration};

use relay_core::platform::CursorControl;
use relay_platform_macos::cursor::MacCursor;
use relay_platform_macos::permissions::check_permissions;

fn main() {
    println!("permissões: {:?}", check_permissions());
    let mut c = MacCursor::default();
    println!("posição inicial: {:?}", c.position());
    c.hide().expect("hide");
    println!("cursor escondido por 2 s");
    sleep(Duration::from_secs(2));
    c.associate(false).expect("associate(false)");
    println!("mouse desassociado: mexa o mouse, o cursor deve ficar parado (2 s)");
    sleep(Duration::from_secs(2));
    c.warp(200, 200).expect("warp");
    println!("warp para (200, 200): {:?}", c.position());
    sleep(Duration::from_secs(1));
    c.associate(true).expect("associate(true)");
    c.show().expect("show");
    println!("restaurado");
}

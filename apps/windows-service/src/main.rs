//! relay-service: `install|uninstall|run|--session-helper`. No Windows é o serviço RelayService
//! (SYSTEM) e seu helper de sessão (item W7); nas demais plataformas só valida a CLI.

mod cli;
#[cfg(windows)]
mod supervisor;

use std::process::ExitCode;

use cli::{Command, parse};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(cmd) => execute(cmd),
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(2)
        }
    }
}

#[cfg(windows)]
fn execute(cmd: Command) -> ExitCode {
    use relay_platform_windows::service;
    let result = match cmd {
        Command::Install => service::install().map(|()| println!("RelayService instalado")),
        Command::Uninstall => service::uninstall().map(|()| println!("RelayService removido")),
        Command::Run => supervisor::run().map_err(|e| e.to_string()),
        Command::SessionHelper => service::run_helper(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn execute(_cmd: Command) -> ExitCode {
    eprintln!("relay-service só funciona no Windows.\n{}", cli::USAGE);
    ExitCode::FAILURE
}

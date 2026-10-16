//! dono: B1
//! Bancada de latência sobre a sessão real (Noise + UDP cifrado). Três modos:
//! - `echo --code C`: escuta, pareia com o código e devolve tudo que recebe nos dois canais.
//! - `send --peer HOST --code C --count N --rate HZ [--json ARQ]`: pareia, mede o RTT de cada canal.
//! - `loopback --count N`: os dois papéis neste processo, em 127.0.0.1.

mod cli;
mod echo;
mod measure;
mod report;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match cli::parse(&args) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{msg}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };
    match run(cmd).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cmd: cli::Command) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        cli::Command::Echo(o) => echo::run_echo(o).await,
        cli::Command::Send(o) => measure::run_send(o).await,
        cli::Command::Loopback(o) => measure::run_loopback(o).await,
    }
}

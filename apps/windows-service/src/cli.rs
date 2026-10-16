//! Parser puro da linha de comando de `relay-service`.

use relay_platform_windows::service::HELPER_ARG;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Install,
    Uninstall,
    /// Entrada do SCM (dispatcher do serviço).
    Run,
    /// Helper lançado pelo serviço na sessão ativa.
    SessionHelper,
}

pub const USAGE: &str = "uso: relay-service <install|uninstall|run|--session-helper>";

/// `args` sem o nome do executável. Exige exatamente um argumento conhecido.
pub fn parse<S: AsRef<str>>(args: &[S]) -> Result<Command, String> {
    match args {
        [a] => match a.as_ref() {
            "install" => Ok(Command::Install),
            "uninstall" => Ok(Command::Uninstall),
            "run" => Ok(Command::Run),
            x if x == HELPER_ARG => Ok(Command::SessionHelper),
            other => Err(format!("comando desconhecido: {other}\n{USAGE}")),
        },
        _ => Err(USAGE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_commands() {
        assert_eq!(parse(&["install"]), Ok(Command::Install));
        assert_eq!(parse(&["uninstall"]), Ok(Command::Uninstall));
        assert_eq!(parse(&["run"]), Ok(Command::Run));
        assert_eq!(parse(&["--session-helper"]), Ok(Command::SessionHelper));
    }

    #[test]
    fn rejects_everything_else() {
        let none: [&str; 0] = [];
        assert!(parse(&none).is_err());
        assert!(parse(&["bogus"]).is_err());
        assert!(parse(&["run", "extra"]).is_err());
    }
}

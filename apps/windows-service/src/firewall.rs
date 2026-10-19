//! Repara as regras de firewall "Relay" a cada início do serviço (SYSTEM). O instalador as cria
//! (installer-hooks.nsh, mesmos argumentos); aqui elas voltam se alguém apagar ou restringir a um
//! perfil: com a rede marcada como Pública, o PC aparecia no mDNS mas o TCP não respondia.
//! Sempre delete + add: são 3 chamadas ao netsh, baratas, e dispensam comparar o estado atual.

use relay_core::types::consts::{PORT_AUDIO_MAC_TO_PC, PORT_CONTROL, PORT_INPUT, PORT_TRANSFER};

fn add_rule(protocol: &str, ports: &str) -> Vec<String> {
    [
        "advfirewall",
        "firewall",
        "add",
        "rule",
        "name=Relay",
        "dir=in",
        "action=allow",
        &format!("protocol={protocol}"),
        &format!("localport={ports}"),
        "profile=any",
        "remoteip=localsubnet",
    ]
    .map(String::from)
    .to_vec()
}

/// Argumentos do netsh na ordem de execução. Devem ficar iguais ao NSIS_HOOK_POSTINSTALL; o teste
/// abaixo confere contra o próprio arquivo do instalador.
pub fn netsh_commands() -> Vec<Vec<String>> {
    vec![
        ["advfirewall", "firewall", "delete", "rule", "name=Relay"]
            .map(String::from)
            .to_vec(),
        add_rule("TCP", &format!("{PORT_CONTROL},{PORT_TRANSFER}")),
        // Faixa contígua: input, áudio PC->Mac e microfone Mac->PC.
        add_rule("UDP", &format!("{PORT_INPUT}-{PORT_AUDIO_MAC_TO_PC}")),
    ]
}

#[cfg(windows)]
pub fn repair() {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    for args in netsh_commands() {
        let out = Command::new("netsh")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match out {
            // O delete falha sem regra existente (primeira execução): esperado.
            Ok(o) if o.status.success() || args[2] == "delete" => {}
            Ok(o) => {
                tracing::warn!(
                    "firewall: netsh {} falhou ({}): {}",
                    args[2],
                    o.status,
                    String::from_utf8_lossy(&o.stdout).trim()
                );
                return;
            }
            Err(e) => {
                tracing::warn!("firewall: não consegui executar o netsh: {e}");
                return;
            }
        }
    }
    tracing::info!("firewall: regras Relay recriadas (profile=any, remoteip=localsubnet)");
}

#[cfg(test)]
mod tests {
    use super::netsh_commands;

    #[test]
    fn regras_iguais_as_do_instalador() {
        let nsis =
            include_str!("../../windows-agent/src-tauri/installer-hooks.nsh").replace('"', "");
        let cmds = netsh_commands();
        assert_eq!(cmds.len(), 3);
        for args in cmds {
            let line = format!("netsh {}", args.join(" "));
            assert!(
                nsis.contains(&line),
                "instalador diverge do serviço: {line}"
            );
        }
    }
}

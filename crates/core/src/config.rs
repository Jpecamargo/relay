//! dono: C9
//! Persistência do Config em TOML: caminho, padrões, migração e gravação atômica.
//! A estrutura `Config` vive em `types::config` (F2).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::types::Config;
use crate::types::config::CONFIG_SCHEMA_VERSION;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Parse(String),
}

fn io_err(e: std::io::Error) -> ConfigError {
    ConfigError::Io(e.to_string())
}

/// `~/Library/Application Support/Relay/config.toml` no Mac; `%APPDATA%\Relay\config.toml` no Windows.
pub fn config_path() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"));
    base.unwrap_or_default().join("Relay").join("config.toml")
}

/// Carrega o arquivo; cria com os padrões se não existe; renomeia para `.bak` se inválido.
pub fn load_or_default(path: &Path) -> Result<Config, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let config = Config::default();
            save(path, &config)?;
            return Ok(config);
        }
        // Conteúdo ilegível (ex.: não UTF-8) conta como arquivo inválido.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => String::new(),
        Err(e) => return Err(io_err(e)),
    };
    match parse(&text) {
        Ok(config) => Ok(config),
        Err(reason) => {
            let bak = bak_path(path);
            tracing::warn!(path = %path.display(), bak = %bak.display(), %reason, "config inválido; usando padrões");
            fs::rename(path, &bak).map_err(io_err)?;
            let config = Config::default();
            save(path, &config)?;
            Ok(config)
        }
    }
}

fn bak_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

fn parse(text: &str) -> Result<Config, String> {
    let mut table: Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    migrate(&mut table);
    let input = Value::Table(table);
    let config: Config = input
        .clone()
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())?;
    let known = Value::try_from(&config).map_err(|e| e.to_string())?;
    for key in unknown_keys(&input, &known) {
        tracing::warn!(%key, "chave desconhecida no config ignorada");
    }
    Ok(config)
}

/// Gancho de migração: leva a tabela da versão em que foi gravada até `CONFIG_SCHEMA_VERSION`.
/// Sem `schema_version` é a v0.
pub fn migrate(table: &mut Table) {
    let mut version = table
        .get("schema_version")
        .and_then(Value::as_integer)
        .unwrap_or(0);
    while version < i64::from(CONFIG_SCHEMA_VERSION) {
        match version {
            // v0 -> v1: o campo `schema_version` passa a existir; o resto do formato é igual.
            0 => {
                table.insert("schema_version".into(), Value::Integer(1));
            }
            // v1 -> v2: o tempo de espera na borda deixa de ser o critério (agora é a pressão,
            // `edge_resistance_pt`). 100 ms era o padrão antigo; outro valor foi escolha do usuário.
            1 => {
                if let Some(Value::Table(t)) = table.get_mut("transition") {
                    if t.get("dwell_ms").and_then(Value::as_integer) == Some(100) {
                        t.insert("dwell_ms".into(), Value::Integer(0));
                    }
                }
                table.insert("schema_version".into(), Value::Integer(2));
            }
            // v2 -> v3: mexia no modo do som do PC no Mac, recurso que saiu na 0.2.0.
            2 => {
                table.insert("schema_version".into(), Value::Integer(3));
            }
            // v3 -> v4: perfis de layout passam a ser do usuário. Antes, cada combinação de telas
            // ganhava sozinha um perfil "Padrão"; fica só o mais completo, como "Principal".
            3 => {
                if let Some(Value::Table(layout)) = table.get_mut("layout")
                    && let Some(Value::Array(profiles)) = layout.get_mut("profiles")
                {
                    let size = |p: &Value| {
                        p.get("displays")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len)
                    };
                    if let Some(mut best) = profiles.iter().max_by_key(|p| size(p)).cloned() {
                        if let Value::Table(t) = &mut best {
                            t.insert(
                                "name".into(),
                                Value::String(crate::layout::DEFAULT_PROFILE_NAME.into()),
                            );
                        }
                        *profiles = vec![best];
                    }
                }
                table.insert("schema_version".into(), Value::Integer(4));
            }
            _ => break,
        }
        version += 1;
    }
}

/// Caminhos (`a.b`) de chaves de tabela presentes em `input` e ausentes em `known`.
/// Listas não são comparadas.
fn unknown_keys(input: &Value, known: &Value) -> Vec<String> {
    fn walk(input: &Table, known: &Table, prefix: &str, out: &mut Vec<String>) {
        for (k, v) in input {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            match known.get(k) {
                None => out.push(path),
                Some(Value::Table(kt)) => {
                    if let Value::Table(it) = v {
                        walk(it, kt, &path, out);
                    }
                }
                Some(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    if let (Value::Table(i), Value::Table(k)) = (input, known) {
        walk(i, k, "", &mut out);
    }
    out
}

/// Grava de forma atômica (temporário + rename).
pub fn save(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let text = toml::to_string_pretty(config).map_err(|e| ConfigError::Parse(e.to_string()))?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let write = || -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ClipboardMode;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn caminho_por_plataforma() {
        let p = config_path();
        #[cfg(target_os = "macos")]
        assert!(p.ends_with("Library/Application Support/Relay/config.toml"));
        #[cfg(windows)]
        assert!(p.ends_with("Relay\\config.toml"));
        assert!(p.ends_with("Relay/config.toml") || p.ends_with("Relay\\config.toml"));
    }

    #[test]
    fn cria_arquivo_com_padroes_quando_nao_existe() {
        let d = dir();
        let p = d.path().join("sub/config.toml");
        let c = load_or_default(&p).unwrap();
        assert_eq!(c, Config::default());
        assert!(p.exists());
        assert_eq!(load_or_default(&p).unwrap(), c);
    }

    #[test]
    fn arquivo_invalido_vira_bak_e_carrega_padroes() {
        let d = dir();
        let p = d.path().join("config.toml");
        fs::write(&p, "isto = = não é toml").unwrap();
        let c = load_or_default(&p).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(
            fs::read_to_string(d.path().join("config.toml.bak")).unwrap(),
            "isto = = não é toml"
        );
        assert_eq!(load_or_default(&p).unwrap(), Config::default());
    }

    #[test]
    fn tipo_errado_tambem_e_invalido() {
        let d = dir();
        let p = d.path().join("config.toml");
        fs::write(
            &p,
            "schema_version = 1\n[transition]\ndwell_ms = \"muito\"\n",
        )
        .unwrap();
        assert_eq!(load_or_default(&p).unwrap(), Config::default());
        assert!(d.path().join("config.toml.bak").exists());
    }

    #[test]
    fn chave_desconhecida_e_ignorada() {
        let d = dir();
        let p = d.path().join("config.toml");
        fs::write(
            &p,
            "schema_version = 1\nfutura = 1\n[transition]\ndwell_ms = 250\nnova = true\n",
        )
        .unwrap();
        let c = load_or_default(&p).unwrap();
        assert_eq!(c.transition.dwell_ms, 250);
        assert!(!d.path().join("config.toml.bak").exists());

        let input = Value::Table(
            "futura = 1\n[transition]\ndwell_ms = 1\nnova = true\n"
                .parse()
                .unwrap(),
        );
        let known = Value::try_from(Config::default()).unwrap();
        assert_eq!(
            unknown_keys(&input, &known),
            vec!["futura", "transition.nova"]
        );
    }

    #[test]
    fn gravacao_atomica_sem_sobra_de_temporario() {
        let d = dir();
        let p = d.path().join("config.toml");
        let mut c = Config::default();
        save(&p, &c).unwrap();
        c.transition.dwell_ms = 321;
        save(&p, &c).unwrap();
        assert_eq!(load_or_default(&p).unwrap().transition.dwell_ms, 321);
        let nomes: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(nomes, vec![std::ffi::OsString::from("config.toml")]);
    }

    #[test]
    fn falha_na_gravacao_preserva_original_e_limpa_temporario() {
        let d = dir();
        let p = d.path().join("config.toml");
        save(&p, &Config::default()).unwrap();
        // rename sobre um diretório falha: o original (outro caminho) fica intacto, sem .tmp.
        let alvo = d.path().join("alvo");
        fs::create_dir(&alvo).unwrap();
        assert!(save(&alvo, &Config::default()).is_err());
        assert!(!d.path().join("alvo.tmp").exists());
        assert_eq!(load_or_default(&p).unwrap(), Config::default());
    }

    #[test]
    fn migra_v0_sem_campo_para_v1() {
        let mut t: Table = "[transition]\ndwell_ms = 200\n".parse().unwrap();
        migrate(&mut t);
        assert_eq!(t["schema_version"].as_integer(), Some(4));

        let d = dir();
        let p = d.path().join("config.toml");
        fs::write(&p, "[transition]\ndwell_ms = 200\n").unwrap();
        let c = load_or_default(&p).unwrap();
        assert_eq!(c.schema_version, 4);
        assert_eq!(c.transition.dwell_ms, 200);
        assert!(!d.path().join("config.toml.bak").exists());
    }

    #[test]
    fn migra_v1_zera_o_dwell_padrao_antigo_e_mantem_o_escolhido() {
        let load = |toml: &str| {
            let d = dir();
            let p = d.path().join("config.toml");
            fs::write(&p, toml).unwrap();
            load_or_default(&p).unwrap()
        };
        let c = load("schema_version = 1\n[transition]\ndwell_ms = 100\n");
        assert_eq!((c.schema_version, c.transition.dwell_ms), (4, 0));
        let c = load("schema_version = 1\n[transition]\ndwell_ms = 250\n");
        assert_eq!((c.schema_version, c.transition.dwell_ms), (4, 250));
        // Já na v2: 100 é escolha do usuário e fica.
        let c = load("schema_version = 2\n[transition]\ndwell_ms = 100\n");
        assert_eq!(c.transition.dwell_ms, 100);
    }

    #[test]
    fn config_com_os_campos_do_som_do_pc_no_mac_ainda_carrega() {
        let d = dir();
        let p = d.path().join("config.toml");
        fs::write(
            &p,
            "schema_version = 2\n[audio]\npc_mode = \"process\"\nprocess_names = [\"Discord.exe\"]\n\
             stream_volume = 0.5\nmac_output_id = \"x\"\njitter_target_ms = 30\n\
             [network.ports]\naudio_pc_to_mac = 1\ncontrol = 9\n",
        )
        .unwrap();
        let c = load_or_default(&p).unwrap();
        assert_eq!(c.schema_version, 4);
        assert_eq!(c.audio.jitter_target_ms, 30);
        assert_eq!(c.network.ports.control, 9);
    }

    #[test]
    fn padroes_do_spec() {
        let c = Config::default();
        let n = &c.network.ports;
        assert_eq!(
            (n.control, n.input, n.audio_mac_to_pc, n.transfer),
            (47800, 47801, 47803, 47804)
        );
        assert_eq!(c.schema_version, 4);
        assert_eq!(c.transition.dwell_ms, 0);
        assert_eq!(c.transition.edge_resistance_pt, 8.0);
        assert_eq!(c.transition.corner_dead_zone_mm, 10.0);
        assert_eq!(c.keymap.cmd_tap_timeout_ms, 400);
        assert_eq!(c.clipboard.mode, ClipboardMode::OnSwitch);
        assert_eq!(c.clipboard.max_bytes, 50 * 1024 * 1024);
        assert_eq!(c.audio.jitter_target_ms, 20);
        assert_eq!(c.hotkeys, crate::types::config::default_hotkeys());
        assert_eq!(c.hotkeys.len(), 5);
    }

    #[test]
    fn sensibilidade_1_0_sobrevive_ao_toml() {
        use crate::types::{LayoutProfile, Machine, PlacedDisplay, ProfileKey, RectMm};
        let d = dir();
        let p = d.path().join("config.toml");
        let mut c = Config::default();
        c.layout.profiles.push(LayoutProfile {
            key: ProfileKey(vec!["a".into()]),
            name: "p".into(),
            displays: vec![PlacedDisplay {
                display_id: "a".into(),
                machine: Machine::Mac,
                rect_mm: RectMm {
                    x: 0.0,
                    y: 0.0,
                    w: 300.0,
                    h: 200.0,
                },
                sensitivity: 1.0,
                rotation: None,
            }],
            windows_detached: false,
        });
        save(&p, &c).unwrap();
        let lido = load_or_default(&p).unwrap();
        assert_eq!(lido, c);
        assert_eq!(lido.layout.profiles[0].displays[0].sensitivity, 1.0);
    }

    #[test]
    fn perfis_automaticos_por_combinacao_de_telas_viram_um_so() {
        let old = "schema_version = 3
[[layout.profiles]]
key = [\"mac\"]
name = \"Padrão\"
windows_detached = false
[[layout.profiles.displays]]
display_id = \"mac\"
machine = \"Mac\"
sensitivity = 1.0
[layout.profiles.displays.rect_mm]
x = 0.0
y = 0.0
w = 300.0
h = 200.0
[[layout.profiles]]
key = [\"mac\", \"w1\"]
name = \"Padrão\"
windows_detached = false
[[layout.profiles.displays]]
display_id = \"mac\"
machine = \"Mac\"
sensitivity = 1.0
[layout.profiles.displays.rect_mm]
x = 0.0
y = 0.0
w = 300.0
h = 200.0
[[layout.profiles.displays]]
display_id = \"w1\"
machine = \"Windows\"
sensitivity = 1.0
[layout.profiles.displays.rect_mm]
x = 300.0
y = 0.0
w = 597.0
h = 336.0
";
        let c = parse(old).unwrap();
        assert_eq!(c.layout.profiles.len(), 1);
        assert_eq!(c.layout.profiles[0].name, "Principal");
        assert_eq!(c.layout.profiles[0].displays.len(), 2);
    }

    #[test]
    fn toml_nao_contem_material_de_chave() {
        let d = dir();
        let p = d.path().join("config.toml");
        save(&p, &Config::default()).unwrap();
        let t = fs::read_to_string(&p).unwrap().to_lowercase();
        // Usos legítimos: as tabelas `[keymap]`/`[[hotkeys]]` e `key = "Digit1"` dos atalhos (KeyboardEvent.code).
        let sem_atalhos: String = t
            .replace("keymap", "")
            .replace("hotkeys", "")
            .lines()
            .filter(|l| !l.trim_start().starts_with("key = \""))
            .collect();
        assert!(!sem_atalhos.contains("key"), "{t}");
        for proibido in [
            "secret",
            "private",
            "psk",
            "passphrase",
            "password",
            "token",
        ] {
            assert!(!t.contains(proibido), "{proibido}");
        }
    }
}

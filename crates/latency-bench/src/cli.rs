//! Argumentos da linha de comando (sem dependência extra).

use std::path::PathBuf;

pub const USAGE: &str = "uso:
  relay-bench echo --code 123456 [--port-control 47800] [--port-input 47801]
  relay-bench send --peer <host[:porta]> --code 123456 [--count 5000] [--rate 500]
                   [--port-input 47801] [--json arquivo]
  relay-bench loopback [--count 10000] [--rate 2000] [--json arquivo]";

pub const DEFAULT_COUNT: u32 = 5000;

pub struct EchoOpts {
    pub code: String,
    pub port_control: u16,
    pub port_input: u16,
}

pub struct SendOpts {
    pub peer: String,
    pub code: String,
    pub count: u32,
    pub rate: u32,
    pub port_control: u16,
    pub port_input: u16,
    pub json: Option<PathBuf>,
}

pub struct LoopbackOpts {
    pub count: u32,
    pub rate: u32,
    pub json: Option<PathBuf>,
}

pub enum Command {
    Echo(EchoOpts),
    Send(SendOpts),
    Loopback(LoopbackOpts),
}

struct Flags(Vec<(String, String)>);

impl Flags {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut out = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let Some(name) = a.strip_prefix("--") else {
                return Err(format!("argumento inesperado: {a}"));
            };
            let v = it.next().ok_or(format!("--{name} pede um valor"))?;
            out.push((name.to_string(), v.clone()));
        }
        Ok(Self(out))
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn num<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.get(name) {
            None => Ok(default),
            Some(v) => v
                .parse()
                .map_err(|_| format!("--{name}: valor inválido: {v}")),
        }
    }

    fn required(&self, name: &str) -> Result<String, String> {
        self.get(name)
            .map(str::to_string)
            .ok_or(format!("falta --{name}"))
    }

    fn known(&self, allowed: &[&str]) -> Result<(), String> {
        for (k, _) in &self.0 {
            if !allowed.contains(&k.as_str()) {
                return Err(format!("opção desconhecida: --{k}"));
            }
        }
        Ok(())
    }
}

fn positive(name: &str, v: u32) -> Result<u32, String> {
    if v == 0 {
        Err(format!("--{name} deve ser maior que 0"))
    } else {
        Ok(v)
    }
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let (mode, rest) = args.split_first().ok_or("falta o modo")?;
    let f = Flags::parse(rest)?;
    let ports = relay_core::types::Ports::default();
    match mode.as_str() {
        "echo" => {
            f.known(&["code", "port-control", "port-input"])?;
            Ok(Command::Echo(EchoOpts {
                code: f.required("code")?,
                port_control: f.num("port-control", ports.control)?,
                port_input: f.num("port-input", ports.input)?,
            }))
        }
        "send" => {
            f.known(&[
                "peer",
                "code",
                "count",
                "rate",
                "port-control",
                "port-input",
                "json",
            ])?;
            Ok(Command::Send(SendOpts {
                peer: f.required("peer")?,
                code: f.required("code")?,
                count: positive("count", f.num("count", DEFAULT_COUNT)?)?,
                rate: positive("rate", f.num("rate", 500)?)?,
                port_control: f.num("port-control", ports.control)?,
                port_input: f.num("port-input", ports.input)?,
                json: f.get("json").map(PathBuf::from),
            }))
        }
        "loopback" => {
            f.known(&["count", "rate", "json"])?;
            Ok(Command::Loopback(LoopbackOpts {
                count: positive("count", f.num("count", 10_000)?)?,
                rate: positive("rate", f.num("rate", 2000)?)?,
                json: f.get("json").map(PathBuf::from),
            }))
        }
        other => Err(format!("modo desconhecido: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_send_with_defaults() {
        let Ok(Command::Send(o)) = parse(&a("send --peer pc.local --code 123456 --rate 250"))
        else {
            panic!("send não parseou");
        };
        assert_eq!((o.peer.as_str(), o.count, o.rate), ("pc.local", 5000, 250));
        assert_eq!((o.port_control, o.port_input), (47800, 47801));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&a("send --code 1")).is_err());
        assert!(parse(&a("loopback --count 0")).is_err());
        assert!(parse(&a("echo --code 1 --nope 2")).is_err());
        assert!(parse(&a("voar")).is_err());
        assert!(parse(&[]).is_err());
    }
}

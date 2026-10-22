//! Command-line parsing, by hand: four commands do not need a crate.

use std::net::SocketAddr;

pub const USAGE: &str = "\
usage:
  mickey target --listen <ip:port>   on the machine to be controlled (Windows)
  mickey source [--to <ip:port>]     on the machine with the keyboard (Linux);
                                     without --to, uses the peers in config.toml
  mickey status [--waybar]           show where the focus is and which PCs are online;
                                     --waybar keeps printing JSON lines for a Waybar module
  mickey id                          show this machine's fingerprint
  mickey forget <fingerprint>        stop trusting a paired machine
";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Id,
    Status {
        waybar: bool,
    },
    Forget {
        fingerprint: String,
    },
    /// `None`: use the peers in `config.toml`.
    Source {
        to: Option<SocketAddr>,
    },
    Target {
        listen: SocketAddr,
    },
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Help),
        [help] if help == "-h" || help == "--help" || help == "help" => Ok(Command::Help),
        [command] if command == "id" => Ok(Command::Id),
        [command] if command == "status" => Ok(Command::Status { waybar: false }),
        [command, flag] if command == "status" && flag == "--waybar" => {
            Ok(Command::Status { waybar: true })
        }
        [command, fingerprint] if command == "forget" => Ok(Command::Forget {
            fingerprint: fingerprint.clone(),
        }),
        [command] if command == "source" => Ok(Command::Source { to: None }),
        [command, flag, addr] if command == "source" && flag == "--to" => Ok(Command::Source {
            to: Some(parse_addr(addr)?),
        }),
        [command, flag, addr] if command == "target" && flag == "--listen" => Ok(Command::Target {
            listen: parse_addr(addr)?,
        }),
        _ => Err(format!(
            "unknown arguments: {}\nrun `mickey --help` for usage",
            args.join(" ")
        )),
    }
}

fn parse_addr(addr: &str) -> Result<SocketAddr, String> {
    addr.parse()
        .map_err(|_| format!("`{addr}` is not an address like 192.0.2.10:4242"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn no_arguments_shows_help() {
        assert_eq!(parse(&[]), Ok(Command::Help));
        assert_eq!(parse(&args("--help")), Ok(Command::Help));
        assert_eq!(parse(&args("id")), Ok(Command::Id));
        assert_eq!(
            parse(&args("status")),
            Ok(Command::Status { waybar: false })
        );
        assert_eq!(
            parse(&args("status --waybar")),
            Ok(Command::Status { waybar: true })
        );
    }

    #[test]
    fn source_and_target_take_an_address() {
        let addr: SocketAddr = "192.0.2.10:4242".parse().unwrap();
        assert_eq!(
            parse(&args("source --to 192.0.2.10:4242")),
            Ok(Command::Source { to: Some(addr) })
        );
        assert_eq!(parse(&args("source")), Ok(Command::Source { to: None }));
        assert_eq!(
            parse(&args("target --listen 192.0.2.10:4242")),
            Ok(Command::Target { listen: addr })
        );
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(parse(&args("source --to nowhere")).is_err());
        assert!(parse(&args("source 192.0.2.10:4242")).is_err());
        assert!(parse(&args("target --to 192.0.2.10:4242")).is_err());
        assert!(parse(&args("fly")).is_err());
        assert!(parse(&args("spike")).is_err());
    }
}

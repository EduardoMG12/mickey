//! Command-line parsing, by hand: four commands do not need a crate.

use std::net::SocketAddr;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage:
  mickey target --listen <ip:port>   on the machine to be controlled (Windows)
  mickey source --to <ip:port>       on the machine with the keyboard (Linux)
  mickey spike [/dev/input/eventN]   M0 spikes: type on Windows, grab on Linux
";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Source { to: SocketAddr },
    Target { listen: SocketAddr },
    Spike { device: Option<PathBuf> },
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Help),
        [help] if help == "-h" || help == "--help" || help == "help" => Ok(Command::Help),
        [command, flag, addr] if command == "source" && flag == "--to" => Ok(Command::Source {
            to: parse_addr(addr)?,
        }),
        [command, flag, addr] if command == "target" && flag == "--listen" => Ok(Command::Target {
            listen: parse_addr(addr)?,
        }),
        [command] if command == "spike" => Ok(Command::Spike { device: None }),
        [command, device] if command == "spike" => Ok(Command::Spike {
            device: Some(PathBuf::from(device)),
        }),
        _ => Err(format!(
            "unknown arguments: {}\nrun `mickey --help` for usage",
            args.join(" ")
        )),
    }
}

fn parse_addr(addr: &str) -> Result<SocketAddr, String> {
    addr.parse()
        .map_err(|_| format!("`{addr}` is not an address like 192.0.2.20:4242"))
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
    }

    #[test]
    fn source_and_target_take_an_address() {
        let addr: SocketAddr = "192.0.2.20:4242".parse().unwrap();
        assert_eq!(
            parse(&args("source --to 192.0.2.20:4242")),
            Ok(Command::Source { to: addr })
        );
        assert_eq!(
            parse(&args("target --listen 192.0.2.20:4242")),
            Ok(Command::Target { listen: addr })
        );
    }

    #[test]
    fn spike_device_is_optional() {
        assert_eq!(parse(&args("spike")), Ok(Command::Spike { device: None }));
        assert_eq!(
            parse(&args("spike /dev/input/event3")),
            Ok(Command::Spike {
                device: Some(PathBuf::from("/dev/input/event3"))
            })
        );
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(parse(&args("source --to nowhere")).is_err());
        assert!(parse(&args("source 192.0.2.20:4242")).is_err());
        assert!(parse(&args("target --to 192.0.2.20:4242")).is_err());
        assert!(parse(&args("fly")).is_err());
    }
}

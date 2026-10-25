//! Command-line parsing, by hand: four commands do not need a crate.

use std::net::SocketAddr;

pub const USAGE: &str = "\
usage:
  mickey run                         do what config.toml says: control other PCs,
                                     be controlled, or both (what the installer starts)
  mickey target --listen <ip:port>   on the machine to be controlled (Windows)
  mickey source [--to <ip:port>]     on the machine with the keyboard (Linux);
                                     without --to, uses the peers in config.toml
  mickey status [--waybar]           show where the focus is and which PCs are online;
                                     --waybar keeps printing JSON lines for a Waybar module
  mickey go <name>                   send the keyboard and mouse to that PC
  mickey home                        bring them back to this PC
  mickey switcher                    show the switcher window
  mickey quit                        end the running Mickey cleanly
  mickey pairing                     let a new PC ask to pair with this one, for 2 minutes
  mickey record                      name the next keys pressed together, or a key
                                     tapped twice (the settings window uses it)
  mickey arranged                    after editing positions in config.toml: use them and
                                     tell the other PCs (the settings window does this)
  mickey settings                    open the settings window
  mickey id [--key]                  show this machine's fingerprint (--key: its public key)
  mickey monitors                    list this machine's monitors
  mickey peers                       list the PCs, whether they are online, and their monitors
  mickey discover [--lines]          list the PCs running Mickey on this network
  mickey autostart [on|off|status]   start Mickey when this PC starts
  mickey connect <ip:port> <name>    add a PC found on the network and pair with it
  mickey history [--lines]           list the recent copies on this PC (newest first);
                                     --lines prints them tab-separated, for the window
  mickey history copy <id>           copy that entry again
  mickey clipboard [test]            say what kind of thing is copied (never what);
                                     test copies a short test text
  mickey forget <fingerprint>        stop trusting a paired machine
  mickey --version                   show which Mickey this is
";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    /// `mickey --version`: which Mickey this is, for bug reports.
    Version,
    Run,
    Monitors,
    /// Ask the running source for the PCs and their monitors.
    Peers,
    /// Ask the running Mickey for the PCs it hears on the network;
    /// `lines`: tab-separated, for the settings window.
    Discover {
        lines: bool,
    },
    /// List the clipboard history; `lines`: tab-separated, for programs.
    History {
        lines: bool,
    },
    /// Say what kind of thing is copied; `write_test` copies a test text.
    Clipboard {
        write_test: bool,
    },
    Settings,
    Id,
    /// `mickey id --key`: this PC's public key, as config.toml writes it
    /// (the settings window marks this PC in "Who listens" with it).
    IdKey,
    /// Start with the system: `Some(on)` sets it, `None` asks.
    Autostart(Option<bool>),
    Status {
        waybar: bool,
    },
    /// An order for the running source: `go <name>`, `home` or `switcher`.
    Order(String),
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
        [version] if version == "-V" || version == "--version" || version == "version" => {
            Ok(Command::Version)
        }
        [command] if command == "id" => Ok(Command::Id),
        [command, flag] if command == "id" && flag == "--key" => Ok(Command::IdKey),
        [command] if command == "run" => Ok(Command::Run),
        [command] if command == "monitors" => Ok(Command::Monitors),
        [command] if command == "peers" => Ok(Command::Peers),
        [command] if command == "autostart" => Ok(Command::Autostart(None)),
        [command, word] if command == "autostart" => match word.as_str() {
            "on" => Ok(Command::Autostart(Some(true))),
            "off" => Ok(Command::Autostart(Some(false))),
            "status" => Ok(Command::Autostart(None)),
            _ => Err(format!("`mickey autostart {word}`: say on, off or status")),
        },
        [command] if command == "discover" => Ok(Command::Discover { lines: false }),
        [command, flag] if command == "discover" && flag == "--lines" => {
            Ok(Command::Discover { lines: true })
        }
        [command, address, name @ ..] if command == "connect" && !name.is_empty() => {
            let address = parse_addr(address)?;
            Ok(Command::Order(format!(
                "connect {address} {}",
                name.join(" ")
            )))
        }
        [command] if command == "history" => Ok(Command::History { lines: false }),
        [command, flag] if command == "history" && flag == "--lines" => {
            Ok(Command::History { lines: true })
        }
        [command, copy, id] if command == "history" && copy == "copy" => {
            let id: u64 = id
                .parse()
                .map_err(|_| format!("{id} is not a history entry number"))?;
            Ok(Command::Order(format!("history copy {id}")))
        }
        [command] if command == "clipboard" => Ok(Command::Clipboard { write_test: false }),
        [command, test] if command == "clipboard" && test == "test" => {
            Ok(Command::Clipboard { write_test: true })
        }
        [command] if command == "settings" => Ok(Command::Settings),
        [command]
            if command == "home"
                || command == "switcher"
                || command == "quit"
                || command == "arranged"
                || command == "record"
                || command == "pairing" =>
        {
            Ok(Command::Order(command.clone()))
        }
        [command, name] if command == "go" => Ok(Command::Order(format!("go {name}"))),
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
        assert_eq!(parse(&args("--version")), Ok(Command::Version));
        assert_eq!(parse(&args("id")), Ok(Command::Id));
        assert_eq!(parse(&args("run")), Ok(Command::Run));
        assert_eq!(parse(&args("home")), Ok(Command::Order("home".into())));
        assert_eq!(
            parse(&["go".into(), "Windows A".into()]),
            Ok(Command::Order("go Windows A".into()))
        );
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

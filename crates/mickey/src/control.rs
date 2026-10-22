//! The control socket of `mickey source` (PRD 0007): a Unix socket that
//! only this user can reach, where small commands ask for the status or
//! follow every change. Nothing here listens on the network.
//!
//! One request line, then answer lines:
//!
//! ```text
//! status       →  one status line, then the connection closes
//! follow       →  a status line now and one more after every change
//! go <name>    →  `ok: …` or `error: …` (focus to that PC)
//! home         →  `ok: …` (focus back to this PC)
//! switcher     →  `ok: …` or `error: …` (show the switcher window)
//! ```
//!
//! The orders only move the focus; nothing on this socket can send input.
//!
//! A status line is tab-separated: the focused PC (0 is this PC), then one
//! `online:<name>` or `offline:<name>` per peer.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A follower that stops reading is dropped instead of slowing the source.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
/// A client that connects and says nothing is dropped too.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a command waits for the source's main loop to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Order {
    Go(String),
    Home,
    Switcher,
    /// Not a move: the answer is `mickey_config::peers::Peers` as text.
    Peers,
}

/// An order for the source's main loop, and where to send its one-line
/// answer (`ok: …` or `error: …`).
pub struct Request {
    pub order: Order,
    pub reply: Sender<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// `None`: this PC's keyboard and mouse are on this PC.
    pub focus: Option<usize>,
    /// Whether the keyboard and mouse in use control this PC now: this
    /// PC's own, or a peer's that came over. The last change wins (PRD
    /// 0009).
    pub here: bool,
    pub peers: Vec<(String, bool)>,
}

impl Status {
    pub fn to_line(&self) -> String {
        let mut line = self.focus.map_or(0, |peer| peer + 1).to_string();
        line.push_str(if self.here { "\there" } else { "\telsewhere" });
        for (name, online) in &self.peers {
            line.push('\t');
            line.push_str(if *online { "online:" } else { "offline:" });
            line.push_str(name);
        }
        line
    }

    pub fn from_line(line: &str) -> Option<Status> {
        let mut fields = line.trim_end_matches('\n').split('\t');
        let focus = match fields.next()?.parse::<usize>().ok()? {
            0 => None,
            card => Some(card - 1),
        };
        let here = match fields.next()? {
            "here" => true,
            "elsewhere" => false,
            _ => return None,
        };
        let peers = fields
            .map(|field| match field.split_once(':')? {
                ("online", name) => Some((name.to_string(), true)),
                ("offline", name) => Some((name.to_string(), false)),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        if focus.is_some_and(|peer| peer >= peers.len()) {
            return None;
        }
        Some(Status { focus, here, peers })
    }

    /// For people: `mickey status`.
    pub fn describe(&self) -> String {
        let focus = match self.focus {
            None => "this PC",
            Some(peer) => &self.peers[peer].0,
        };
        let mut text = format!("focus: {focus}\n");
        let control = if self.here { "this PC" } else { "another PC" };
        text.push_str(&format!("in control: {control}\n"));
        for (name, online) in &self.peers {
            let state = if *online { "online" } else { "offline" };
            text.push_str(&format!("{name}: {state}\n"));
        }
        text
    }
}

/// One line for a Waybar `custom` module with `"return-type": "json"`. The
/// `class` lets the bar's CSS color it: `local`, `remote`, or `off` when no
/// source is running (`status` is `None`).
pub fn waybar_line(status: Option<&Status>) -> String {
    let Some(status) = status else {
        return r#"{"text":"Mickey off","tooltip":"mickey source is not running","class":"off"}"#
            .to_string();
    };
    let online = status.peers.iter().filter(|(_, online)| *online).count();
    let (focus, class) = match status.focus {
        None => ("This PC", "local"),
        Some(peer) => (status.peers[peer].0.as_str(), "remote"),
    };
    let text = format!("{focus} · {online}/{}", status.peers.len());
    let tooltip = status.describe();
    format!(
        r#"{{"text":"{}","tooltip":"{}","class":"{class}"}}"#,
        json_escape(&text),
        json_escape(tooltip.trim_end())
    )
}

/// Escapes a string for the inside of a JSON string literal.
fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

/// Where the control channel lives and how to reach it. Unix: a socket in
/// the user's runtime directory (0700). Windows, which has no Unix sockets
/// in std: TCP on 127.0.0.1 with a random port and a random token, both in
/// a file in the user's own %LOCALAPPDATA% that a client must read first.
#[cfg(unix)]
mod endpoint {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    pub type Stream = UnixStream;

    pub struct Endpoint {
        path: PathBuf,
    }

    pub struct Listener(UnixListener);

    impl Endpoint {
        /// `$XDG_RUNTIME_DIR/mickey/control.sock`.
        pub fn default() -> Result<Endpoint, String> {
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .ok_or("XDG_RUNTIME_DIR is not set, so there is no control socket")?;
            Ok(Endpoint::at(runtime.join("mickey").join("control.sock")))
        }

        pub fn at(path: PathBuf) -> Endpoint {
            Endpoint { path }
        }

        pub fn connect(&self) -> io::Result<Stream> {
            UnixStream::connect(&self.path)
        }

        pub fn listen(&self) -> Result<Listener, String> {
            let dir = self.path.parent().expect("the socket path has a directory");
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
            if self.connect().is_ok() {
                return Err("another mickey source is already running".into());
            }
            // A socket file left by a crash answers nobody; replace it.
            let _ = fs::remove_file(&self.path);
            UnixListener::bind(&self.path)
                .map(Listener)
                .map_err(|err| format!("cannot listen on {}: {err}", self.path.display()))
        }
    }

    impl Listener {
        pub fn incoming(&self) -> impl Iterator<Item = Stream> + '_ {
            self.0.incoming().flatten()
        }

        /// The directory's permissions already keep other users out.
        pub fn admit(&self, _reader: &mut impl BufRead) -> bool {
            true
        }
    }
}

#[cfg(windows)]
mod endpoint {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener, TcpStream};

    pub type Stream = TcpStream;

    pub struct Endpoint {
        /// Holds "<port> <token>".
        file: PathBuf,
    }

    pub struct Listener {
        inner: TcpListener,
        token: String,
    }

    impl Endpoint {
        /// `%LOCALAPPDATA%\mickey\control.txt`.
        pub fn default() -> Result<Endpoint, String> {
            let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
            Ok(Endpoint::at(
                PathBuf::from(local).join("mickey").join("control.txt"),
            ))
        }

        pub fn at(file: PathBuf) -> Endpoint {
            Endpoint { file }
        }

        fn read(&self) -> io::Result<(u16, String)> {
            let text = fs::read_to_string(&self.file)?;
            let (port, token) = text
                .trim()
                .split_once(' ')
                .ok_or_else(|| io::Error::other("damaged control file"))?;
            let port = port.parse().map_err(io::Error::other)?;
            Ok((port, token.to_string()))
        }

        pub fn connect(&self) -> io::Result<Stream> {
            let (port, _) = self.read()?;
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
            introduce(&mut stream, self)?;
            Ok(stream)
        }

        pub fn listen(&self) -> Result<Listener, String> {
            if self.connect().is_ok() {
                return Err("another mickey source is already running".into());
            }
            let inner = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .map_err(|err| format!("cannot listen on localhost: {err}"))?;
            let port = inner.local_addr().map_err(|err| err.to_string())?.port();
            let token = random_token()?;
            if let Some(dir) = self.file.parent() {
                fs::create_dir_all(dir).map_err(|err| err.to_string())?;
            }
            fs::write(&self.file, format!("{port} {token}"))
                .map_err(|err| format!("cannot write {}: {err}", self.file.display()))?;
            Ok(Listener { inner, token })
        }
    }

    impl Listener {
        pub fn incoming(&self) -> impl Iterator<Item = Stream> + '_ {
            self.inner.incoming().flatten()
        }

        /// Only a client that read the token from the user's own file may
        /// talk: other users, and web pages reaching localhost, cannot.
        pub fn admit(&self, reader: &mut impl BufRead) -> bool {
            let mut line = String::new();
            reader.read_line(&mut line).is_ok() && line.trim() == self.token
        }
    }

    /// Sends the token before the request.
    fn introduce(stream: &mut Stream, endpoint: &Endpoint) -> io::Result<()> {
        let (_, token) = endpoint.read()?;
        writeln!(stream, "{token}")
    }

    /// 32 random bytes as hex, from the same generator the identity keys
    /// use (snow, through the OS), so no new dependency.
    fn random_token() -> Result<String, String> {
        let params = crate::identity::NOISE_PARAMS
            .parse()
            .map_err(|err| format!("{err}"))?;
        let keypair = snow::Builder::new(params)
            .generate_keypair()
            .map_err(|err| format!("{err}"))?;
        Ok(keypair
            .private
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

use endpoint::{Endpoint, Stream};

/// Whether another `mickey source` answers on the control channel.
pub fn source_running() -> bool {
    Endpoint::default().is_ok_and(|endpoint| endpoint.connect().is_ok())
}

/// The source's end: publish a status whenever it changes.
pub struct Control {
    updates: Sender<Status>,
}

impl Control {
    /// Listens on the control channel. Fails if another `mickey source`
    /// already owns it.
    /// Orders arrive on `requests`, as any type that can be made from a
    /// `Request`, so the source can share one channel with its input.
    pub fn start<T: From<Request> + Send + 'static>(
        requests: Sender<T>,
    ) -> Result<Control, String> {
        Control::start_at(Endpoint::default()?, requests)
    }

    fn start_at<T: From<Request> + Send + 'static>(
        endpoint: Endpoint,
        requests: Sender<T>,
    ) -> Result<Control, String> {
        let listener = endpoint.listen()?;
        let latest = Arc::new(Mutex::new(None::<Status>));
        let (followers_in, followers) = mpsc::channel();
        let (updates, updates_out) = mpsc::channel();
        let shared = Arc::clone(&latest);
        thread::spawn(move || accept(listener, &shared, &followers_in, &requests));
        thread::spawn(move || publish(updates_out, followers, &latest));
        Ok(Control { updates })
    }

    pub fn publish(&self, status: Status) {
        // Only fails if the publisher thread is gone; then nobody listens.
        let _ = self.updates.send(status);
    }
}

fn accept<T: From<Request>>(
    listener: endpoint::Listener,
    latest: &Mutex<Option<Status>>,
    followers: &Sender<Stream>,
    requests: &Sender<T>,
) {
    for stream in listener.incoming() {
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        // One reader for both lines, so nothing read ahead is lost.
        let mut reader = BufReader::new(&stream);
        let mut request = String::new();
        if !listener.admit(&mut reader) || reader.read_line(&mut request).is_err() {
            continue;
        }
        drop(reader);
        let mut stream = stream;
        let status = latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match request.trim() {
            "status" => {
                if let Some(status) = status {
                    let _ = writeln!(stream, "{}", status.to_line());
                }
            }
            "follow" => {
                if let Some(status) = status {
                    let _ = writeln!(stream, "{}", status.to_line());
                }
                let _ = followers.send(stream);
            }
            other => {
                let order = match other.split_once(' ') {
                    _ if other == "home" => Some(Order::Home),
                    _ if other == "switcher" => Some(Order::Switcher),
                    _ if other == "peers" => Some(Order::Peers),
                    Some(("go", name)) => Some(Order::Go(name.to_string())),
                    _ => None,
                };
                let answer = match order {
                    None => format!("error: unknown request `{other}`"),
                    Some(order) => {
                        let (reply, answer) = mpsc::channel();
                        let _ = requests.send(T::from(Request { order, reply }));
                        answer
                            .recv_timeout(ANSWER_TIMEOUT)
                            .unwrap_or_else(|_| "error: mickey source did not answer".into())
                    }
                };
                let _ = writeln!(stream, "{answer}");
            }
        }
    }
}

fn publish(
    updates: Receiver<Status>,
    new_followers: Receiver<Stream>,
    latest: &Mutex<Option<Status>>,
) {
    let mut followers: Vec<Stream> = Vec::new();
    for status in updates {
        *latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(status.clone());
        followers.extend(new_followers.try_iter());
        let line = format!("{}\n", status.to_line());
        followers.retain_mut(|stream| stream.write_all(line.as_bytes()).is_ok());
    }
}

/// The command's end: one request, the answer lines as they come.
pub fn request(request: &str) -> Result<impl Iterator<Item = io::Result<String>>, String> {
    request_at(&Endpoint::default()?, request)
}

fn request_at(
    endpoint: &Endpoint,
    request: &str,
) -> Result<impl Iterator<Item = io::Result<String>> + use<>, String> {
    let mut stream = endpoint
        .connect()
        .map_err(|_| "mickey source is not running (no control channel)".to_string())?;
    // Followers wait for changes for as long as they like.
    stream
        .set_read_timeout(None)
        .map_err(|err| err.to_string())?;
    writeln!(stream, "{request}").map_err(|err| err.to_string())?;
    Ok(BufReader::new(stream).lines())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            focus: Some(1),
            here: false,
            peers: vec![("Windows A".into(), true), ("Win: B".into(), false)],
        }
    }

    #[test]
    fn a_status_line_round_trips() {
        let line = status().to_line();
        assert_eq!(line, "2\telsewhere\tonline:Windows A\toffline:Win: B");
        assert_eq!(Status::from_line(&line), Some(status()));
    }

    #[test]
    fn bad_lines_are_rejected() {
        assert_eq!(Status::from_line(""), None);
        assert_eq!(Status::from_line("x"), None);
        assert_eq!(
            Status::from_line("2\telsewhere\tonline:A"),
            None,
            "focus past the peers"
        );
        assert_eq!(Status::from_line("0\there\tmaybe:A"), None);
        assert_eq!(Status::from_line("0"), None, "no here or elsewhere");
        assert_eq!(
            Status::from_line("0\there"),
            Some(Status {
                focus: None,
                here: true,
                peers: vec![]
            })
        );
    }

    #[test]
    fn describe_is_for_people() {
        assert_eq!(
            status().describe(),
            "focus: Win: B\nin control: another PC\nWindows A: online\nWin: B: offline\n"
        );
    }

    #[test]
    fn waybar_lines_are_json_with_a_class() {
        assert_eq!(
            waybar_line(Some(&status())),
            r#"{"text":"Win: B · 1/2","tooltip":"focus: Win: B\nin control: another PC\nWindows A: online\nWin: B: offline","class":"remote"}"#
        );
        let home = Status {
            focus: None,
            ..status()
        };
        assert!(waybar_line(Some(&home)).contains(r#""class":"local""#));
        assert!(waybar_line(None).contains(r#""class":"off""#));
    }

    #[test]
    fn json_escape_handles_quotes_backslashes_and_controls() {
        assert_eq!(json_escape("a\"b\\c\nd\u{1}"), r#"a\"b\\c\nd\u0001"#);
    }

    #[test]
    fn status_and_follow_over_a_real_socket() {
        let dir = std::env::temp_dir().join(format!("mickey-control-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("mickey").join("control");

        let (requests, orders) = mpsc::channel::<Request>();
        let control = Control::start_at(Endpoint::at(path.clone()), requests.clone()).unwrap();
        assert!(
            Control::start_at(Endpoint::at(path.clone()), requests).is_err(),
            "a second source is refused"
        );
        control.publish(status());
        thread::sleep(Duration::from_millis(50));

        let mut answer = request_at(&Endpoint::at(path.clone()), "status").unwrap();
        assert_eq!(answer.next().unwrap().unwrap(), status().to_line());
        assert!(answer.next().is_none(), "status closes after one line");

        let mut follow = request_at(&Endpoint::at(path.clone()), "follow").unwrap();
        assert_eq!(follow.next().unwrap().unwrap(), status().to_line());
        let home = Status {
            focus: None,
            ..status()
        };
        control.publish(home.clone());
        assert_eq!(follow.next().unwrap().unwrap(), home.to_line());

        let mut bad = request_at(&Endpoint::at(path.clone()), "dance").unwrap();
        assert!(bad.next().unwrap().unwrap().starts_with("error"));

        // An order reaches the main loop, and its answer reaches the client.
        let client = thread::spawn(move || {
            request_at(&Endpoint::at(path), "go Windows A")
                .unwrap()
                .next()
        });
        let order = orders.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(order.order, Order::Go("Windows A".into()));
        order.reply.send("ok: going".into()).unwrap();
        assert_eq!(client.join().unwrap().unwrap().unwrap(), "ok: going");
        fs::remove_dir_all(&dir).unwrap();
    }
}

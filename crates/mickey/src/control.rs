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
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const SOCKET: &str = "control.sock";
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
}

/// An order for the source's main loop, and where to send its one-line
/// answer (`ok: …` or `error: …`).
pub struct Request {
    pub order: Order,
    pub reply: Sender<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// `None`: the focus is on this PC.
    pub focus: Option<usize>,
    pub peers: Vec<(String, bool)>,
}

impl Status {
    pub fn to_line(&self) -> String {
        let mut line = self.focus.map_or(0, |peer| peer + 1).to_string();
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
        Some(Status { focus, peers })
    }

    /// For people: `mickey status`.
    pub fn describe(&self) -> String {
        let focus = match self.focus {
            None => "this PC",
            Some(peer) => &self.peers[peer].0,
        };
        let mut text = format!("focus: {focus}\n");
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

/// `$XDG_RUNTIME_DIR/mickey/control.sock`, the per-user runtime directory
/// that only exists while the user is logged in.
pub fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .ok_or("XDG_RUNTIME_DIR is not set, so there is no control socket")?;
    Ok(runtime.join("mickey").join(SOCKET))
}

/// Whether another `mickey source` answers on the control socket.
pub fn source_running() -> bool {
    socket_path().is_ok_and(|path| UnixStream::connect(path).is_ok())
}

/// The source's end: publish a status whenever it changes.
pub struct Control {
    updates: Sender<Status>,
}

impl Control {
    /// Listens on the control socket. Fails if another `mickey source`
    /// already owns it.
    /// Orders arrive on `requests`, as any type that can be made from a
    /// `Request`, so the source can share one channel with its input.
    pub fn start<T: From<Request> + Send + 'static>(
        requests: Sender<T>,
    ) -> Result<Control, String> {
        Control::start_at(socket_path()?, requests)
    }

    fn start_at<T: From<Request> + Send + 'static>(
        path: PathBuf,
        requests: Sender<T>,
    ) -> Result<Control, String> {
        let dir = path.parent().expect("the socket path has a directory");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        if UnixStream::connect(&path).is_ok() {
            return Err("another mickey source is already running".into());
        }
        // A socket file left by a crash answers nobody; replace it.
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)
            .map_err(|err| format!("cannot listen on {}: {err}", path.display()))?;

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
    listener: UnixListener,
    latest: &Mutex<Option<Status>>,
    followers: &Sender<UnixStream>,
    requests: &Sender<T>,
) {
    for stream in listener.incoming().flatten() {
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
        let mut request = String::new();
        if BufReader::new(&stream).read_line(&mut request).is_err() {
            continue;
        }
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
    new_followers: Receiver<UnixStream>,
    latest: &Mutex<Option<Status>>,
) {
    let mut followers: Vec<UnixStream> = Vec::new();
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
    request_at(&socket_path()?, request)
}

fn request_at(
    path: &std::path::Path,
    request: &str,
) -> Result<impl Iterator<Item = io::Result<String>> + use<>, String> {
    let mut stream = UnixStream::connect(path)
        .map_err(|_| "mickey source is not running (no control socket)".to_string())?;
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
            peers: vec![("Windows A".into(), true), ("Win: B".into(), false)],
        }
    }

    #[test]
    fn a_status_line_round_trips() {
        let line = status().to_line();
        assert_eq!(line, "2\tonline:Windows A\toffline:Win: B");
        assert_eq!(Status::from_line(&line), Some(status()));
    }

    #[test]
    fn bad_lines_are_rejected() {
        assert_eq!(Status::from_line(""), None);
        assert_eq!(Status::from_line("x"), None);
        assert_eq!(
            Status::from_line("2\tonline:A"),
            None,
            "focus past the peers"
        );
        assert_eq!(Status::from_line("0\tmaybe:A"), None);
        assert_eq!(
            Status::from_line("0"),
            Some(Status {
                focus: None,
                peers: vec![]
            })
        );
    }

    #[test]
    fn describe_is_for_people() {
        assert_eq!(
            status().describe(),
            "focus: Win: B\nWindows A: online\nWin: B: offline\n"
        );
    }

    #[test]
    fn waybar_lines_are_json_with_a_class() {
        assert_eq!(
            waybar_line(Some(&status())),
            r#"{"text":"Win: B · 1/2","tooltip":"focus: Win: B\nWindows A: online\nWin: B: offline","class":"remote"}"#
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
        let path = dir.join("mickey").join(SOCKET);

        let (requests, orders) = mpsc::channel::<Request>();
        let control = Control::start_at(path.clone(), requests.clone()).unwrap();
        assert!(
            Control::start_at(path.clone(), requests).is_err(),
            "a second source is refused"
        );
        control.publish(status());
        thread::sleep(Duration::from_millis(50));

        let mut answer = request_at(&path, "status").unwrap();
        assert_eq!(answer.next().unwrap().unwrap(), status().to_line());
        assert!(answer.next().is_none(), "status closes after one line");

        let mut follow = request_at(&path, "follow").unwrap();
        assert_eq!(follow.next().unwrap().unwrap(), status().to_line());
        let home = Status {
            focus: None,
            ..status()
        };
        control.publish(home.clone());
        assert_eq!(follow.next().unwrap().unwrap(), home.to_line());

        let mut bad = request_at(&path, "dance").unwrap();
        assert!(bad.next().unwrap().unwrap().starts_with("error"));

        // An order reaches the main loop, and its answer reaches the client.
        let client = thread::spawn(move || request_at(&path, "go Windows A").unwrap().next());
        let order = orders.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(order.order, Order::Go("Windows A".into()));
        order.reply.send("ok: going".into()).unwrap();
        assert_eq!(client.join().unwrap().unwrap().unwrap(), "ok: going");
        fs::remove_dir_all(&dir).unwrap();
    }
}

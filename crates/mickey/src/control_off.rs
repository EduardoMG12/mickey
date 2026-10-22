//! The control socket where it does not exist yet: on Windows. Same types
//! as `control.rs`, but `Control::start` always fails, so `mickey source`
//! runs without bars and commands there.
// ponytail: Windows has no Unix sockets in std; a named pipe would give it
// `mickey status` and the rest later.

use std::sync::mpsc::Sender;

// Never built here: orders only come through the socket.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Order {
    Go(String),
    Home,
    Switcher,
}

pub struct Request {
    pub order: Order,
    pub reply: Sender<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub focus: Option<usize>,
    pub peers: Vec<(String, bool)>,
}

pub struct Control;

impl Control {
    pub fn start<T: From<Request> + Send + 'static>(
        _requests: Sender<T>,
    ) -> Result<Control, String> {
        Err("not available on this system yet".into())
    }

    pub fn publish(&self, _status: Status) {}
}

pub fn source_running() -> bool {
    false
}

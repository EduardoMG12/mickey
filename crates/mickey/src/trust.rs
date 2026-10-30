//! Trusted peers (PRD 0002): the public keys this machine has paired with.
//! A peer is trusted by its key, never by its address.
//!
//! The file `trusted_peers` sits next to the identity, one peer per line:
//!
//! ```text
//! <public key, 64 hex digits> <where it was paired from>
//! ```

use crate::identity::fingerprint;
use crate::threads::LockAnyway;
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "trusted_peers";

/// The one list a running Mickey keeps: its source and its target both
/// read it, and what one learns (a pairing, a PC the group introduced) the
/// other sees at once.
pub type Shared = std::sync::Arc<std::sync::Mutex<TrustedPeers>>;

/// Where the list lives: the user's data folder, except on Windows when
/// this Mickey may run as administrator (installed where only
/// administrators can change it). Then it is `state` next to mickey.exe,
/// which only administrators can change: in %APPDATA% any program of the
/// user could add its own key, and the administrator Mickey would type
/// what it sends into administrator windows. That list starts empty: one
/// in %APPDATA% may already have been changed, so PCs pair again.
pub fn dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    if let Some(state) = admin_state_dir() {
        return Ok(state);
    }
    crate::identity::data_dir()
}

/// Whether this process may change the list itself; if not, the running
/// Mickey does (`forget` through the control channel).
pub fn can_change() -> bool {
    #[cfg(windows)]
    if admin_state_dir().is_some() && !crate::platform::acl::elevated() {
        return false;
    }
    true
}

/// `state` next to mickey.exe, made by Mickey running as administrator,
/// when only administrators can change this Mickey and that folder.
#[cfg(windows)]
fn admin_state_dir() -> Option<PathBuf> {
    if !crate::autostart::install_is_safe() {
        return None;
    }
    let state = std::env::current_exe().ok()?.with_file_name("state");
    if crate::platform::acl::elevated() {
        fs::create_dir_all(&state).ok()?;
    }
    let safe = crate::platform::acl::admin_only(&state).unwrap_or(false);
    (state.is_dir() && safe).then_some(state)
}

/// Loads the list once, to share.
pub fn shared(dir: &Path) -> Result<Shared, String> {
    Ok(std::sync::Arc::new(std::sync::Mutex::new(
        TrustedPeers::load(dir)?,
    )))
}

pub struct TrustedPeers {
    path: PathBuf,
    peers: Vec<([u8; 32], String)>,
}

impl TrustedPeers {
    /// Reads the list; a missing file means nobody is trusted yet. A damaged
    /// line is an error rather than skipped, so a typo never goes unnoticed.
    pub fn load(dir: &Path) -> Result<TrustedPeers, String> {
        let path = dir.join(FILE_NAME);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
        };
        let mut peers = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, note) = line.split_once(' ').unwrap_or((line, ""));
            let key = crate::hex::decode_key(key)
                .ok_or_else(|| format!("{} line {} is damaged", path.display(), number + 1))?;
            peers.push((key, note.to_string()));
        }
        Ok(TrustedPeers { path, peers })
    }

    /// Whether no PC is trusted yet.
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    pub fn is_trusted(&self, key: &[u8; 32]) -> bool {
        self.peers.iter().any(|(trusted, _)| trusted == key)
    }

    /// Adds `key` and saves the list. `note` is informational, e.g. the
    /// address the peer was paired from.
    pub fn add(&mut self, key: [u8; 32], note: &str) -> Result<(), String> {
        if !self.is_trusted(&key) {
            self.peers.push((key, note.to_string()));
        }
        self.save()
    }

    /// Removes the peer with this fingerprint (dashes and case ignored),
    /// and every PC it introduced through the group (noted `via <its
    /// fingerprint>`, #184). Returns whether one was found.
    pub fn forget(&mut self, wanted: &str) -> Result<bool, String> {
        let normalize = |text: &str| text.replace('-', "").to_ascii_uppercase();
        // A public key (64 hex digits, as config.toml and the settings keep
        // it) stands for its fingerprint.
        let wanted = match crate::hex::decode_key(wanted) {
            Some(key) => normalize(&fingerprint(&key)),
            None => normalize(wanted),
        };
        let before = self.peers.len();
        let introduced = |note: &str| {
            note.split_once(" via ")
                .is_some_and(|(_, by)| normalize(by.trim()) == wanted)
        };
        self.peers
            .retain(|(key, note)| normalize(&fingerprint(key)) != wanted && !introduced(note));
        let found = self.peers.len() != before;
        if found {
            self.save()?;
        }
        Ok(found)
    }

    fn save(&self) -> Result<(), String> {
        let error = |err: std::io::Error| format!("cannot save {}: {err}", self.path.display());
        let mut text = String::new();
        for (key, note) in &self.peers {
            text.push_str(&crate::hex::encode(key));
            text.push(' ');
            text.push_str(note);
            text.push('\n');
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&self.path).map_err(error)?;
        // A list made before with wider rights keeps them unless narrowed:
        // `mode` above only applies to a new file.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(error)?;
        }
        (&file).write_all(text.as_bytes()).map_err(error)
    }
}

/// Whether to talk to the peer whose key is `peer_key`. A trusted key is
/// accepted at once; an unknown one only after the person at this machine
/// sees the same pairing `code` on both PCs and pairs, in a window or else
/// on the terminal. Anything else refuses. `who` names the other PC.
pub fn trust_or_ask(
    trusted: &Shared,
    peer_key: &[u8; 32],
    address: SocketAddr,
    code: &str,
    who: &str,
) -> Result<bool, String> {
    let lock = || trusted.lock_anyway();
    if lock().is_trusted(peer_key) {
        return Ok(true);
    }
    // The list is not held while the person decides.
    if !ask_once(peer_key, address, code, who)? {
        return Ok(false);
    }
    lock().add(*peer_key, &address.to_string())?;
    println!("mickey: paired; it will connect without asking from now on");
    Ok(true)
}

/// How long pairing stays open after the owner opens it (#181).
pub const PAIRING_OPEN: std::time::Duration = std::time::Duration::from_secs(120);

/// Until when a PC that was never paired may ask to pair.
static OPEN_UNTIL: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Lets a new PC ask to pair for `PAIRING_OPEN`: from the settings, the
/// command line, or a Connect the owner just clicked.
pub fn open_pairing() {
    *OPEN_UNTIL.lock_anyway() = Some(std::time::Instant::now() + PAIRING_OPEN);
}

/// Whether a PC never paired may ask to pair now: while this PC trusts
/// nobody yet (a fresh install pairs with no extra step), or while the
/// owner has opened pairing. Anything else is refused without a window.
pub fn pairing_open(trusted: &std::sync::Mutex<TrustedPeers>) -> bool {
    let fresh = trusted.lock_anyway().is_empty();
    let until = *OPEN_UNTIL.lock_anyway();
    fresh || until.is_some_and(|until| std::time::Instant::now() < until)
}

/// How long a PC whose request was refused must wait to ask again.
const REFUSED_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether a pairing question is on screen now: one at a time.
static ASKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// When each address was last refused.
static REFUSED: std::sync::Mutex<Vec<(std::net::IpAddr, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());

/// How long until `ip`, refused a moment ago, may ask to pair again; `None`
/// when it may now. `mickey connect` says so instead of a code that would
/// never show.
pub fn refused_for(ip: std::net::IpAddr) -> Option<std::time::Duration> {
    let now = std::time::Instant::now();
    REFUSED
        .lock_anyway()
        .iter()
        .filter(|(known, _)| *known == ip)
        .map(|(_, at)| REFUSED_WAIT.saturating_sub(now.duration_since(*at)))
        .max()
        .filter(|left| !left.is_zero())
}

/// Whether `ip` was refused less than `REFUSED_WAIT` before `now`; older
/// refusals are forgotten on the way.
fn refused_recently(
    refused: &mut Vec<(std::net::IpAddr, std::time::Instant)>,
    ip: std::net::IpAddr,
    now: std::time::Instant,
) -> bool {
    refused.retain(|(_, at)| now.duration_since(*at) < REFUSED_WAIT);
    refused.iter().any(|(known, _)| *known == ip)
}

/// `ask`, guarded against a PC on the network that is not paired opening
/// window after window (#181): one question at a time, the others refused
/// at once; an address refused waits `REFUSED_WAIT`; the address shows
/// beside the name, which anyone can announce.
pub fn ask_once(
    peer_key: &[u8; 32],
    address: SocketAddr,
    code: &str,
    who: &str,
) -> Result<bool, String> {
    use std::sync::atomic::Ordering;
    let ip = address.ip();
    {
        let mut refused = REFUSED.lock_anyway();
        if refused_recently(&mut refused, ip, std::time::Instant::now()) {
            return Ok(false);
        }
    }
    if ASKING.swap(true, Ordering::SeqCst) {
        println!("mickey: {address} asked to pair while another PC was asking; refused");
        return Ok(false);
    }
    let shown = if who == ip.to_string() {
        who.to_string()
    } else {
        format!("{who} ({ip})")
    };
    let answer = ask(peer_key, address, code, &shown);
    ASKING.store(false, Ordering::SeqCst);
    if answer != Ok(true) {
        REFUSED.lock_anyway().push((ip, std::time::Instant::now()));
    }
    answer
}

/// Asks the person at this PC whether to pair with the PC whose key is
/// `peer_key`: in a window, or on the terminal when there is no window.
fn ask(peer_key: &[u8; 32], address: SocketAddr, code: &str, who: &str) -> Result<bool, String> {
    println!("mickey: new PC at {address} ({})", fingerprint(peer_key));
    println!("mickey: pairing code: {code}");
    match ask_in_window(who, code) {
        Some(answer) => Ok(answer),
        None => ask_in_terminal(),
    }
}

/// Asks with `mickey-ui pair` (PRD 0010), installed next to this program;
/// `None` when it cannot show a window, so the terminal is asked instead.
fn ask_in_window(who: &str, code: &str) -> Option<bool> {
    let status = crate::window::ui_command(&["pair", who, code])
        .and_then(|mut ui| ui.status())
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        // 2: no display to open a window on.
        _ => None,
    }
}

fn ask_in_terminal() -> Result<bool, String> {
    println!("mickey: pair only if the other PC shows the same code.");
    print!("mickey: type y to pair, anything else to refuse: ");
    std::io::Write::flush(&mut std::io::stdout()).map_err(|err| err.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|err| format!("cannot read the answer: {err}"))?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    /// Run by hand on Windows, from a copy inside Program Files and from
    /// one outside it, elevated and not: prints where the list lives.
    #[cfg(windows)]
    #[test]
    #[ignore = "depends on where the test runs from; run by hand on Windows"]
    fn where_the_list_lives() {
        println!(
            "elevated {} | safe {} | dir {} | can change {}",
            crate::platform::acl::elevated(),
            crate::autostart::install_is_safe(),
            super::dir().unwrap().display(),
            super::can_change()
        );
    }

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mickey-trust-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nobody_is_trusted_at_first_and_added_peers_persist() {
        let dir = temp_dir("persist");
        let mut trusted = TrustedPeers::load(&dir).unwrap();
        assert!(!trusted.is_trusted(&[1; 32]));
        trusted.add([1; 32], "192.0.2.10").unwrap();
        let reloaded = TrustedPeers::load(&dir).unwrap();
        assert!(reloaded.is_trusted(&[1; 32]));
        assert!(!reloaded.is_trusted(&[2; 32]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saving_narrows_a_list_others_could_read() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("narrow");
        let path = dir.join(FILE_NAME);
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut trusted = TrustedPeers::load(&dir).unwrap();
        trusted.add([7; 32], "test").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn forget_takes_a_public_key_too() {
        let dir = temp_dir("forget-key");
        let mut trusted = TrustedPeers::load(&dir).unwrap();
        trusted.add([3; 32], "").unwrap();
        trusted.add([4; 32], "").unwrap();
        // As config.toml and the settings keep it (the settings' Remove).
        assert!(trusted.forget(&crate::hex::encode(&[3; 32])).unwrap());
        assert!(!trusted.is_trusted(&[3; 32]));
        assert!(trusted.is_trusted(&[4; 32]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forget_takes_a_fingerprint_with_or_without_dashes() {
        let dir = temp_dir("forget");
        let mut trusted = TrustedPeers::load(&dir).unwrap();
        trusted.add([1; 32], "").unwrap();
        trusted.add([2; 32], "").unwrap();
        let plain = fingerprint(&[1; 32]).replace('-', "").to_lowercase();
        assert!(trusted.forget(&plain).unwrap());
        assert!(!trusted.forget(&plain).unwrap());
        let reloaded = TrustedPeers::load(&dir).unwrap();
        assert!(!reloaded.is_trusted(&[1; 32]));
        assert!(reloaded.is_trusted(&[2; 32]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forgetting_a_pc_forgets_the_pcs_it_introduced() {
        let dir = temp_dir("cascade");
        let mut trusted = TrustedPeers::load(&dir).unwrap();
        trusted.add([1; 32], "192.0.2.1:4242").unwrap();
        let by_one = format!("192.0.2.2:4242 via {}", fingerprint(&[1; 32]));
        trusted.add([2; 32], &by_one).unwrap();
        trusted.add([3; 32], "192.0.2.3:4242").unwrap();
        assert!(trusted.forget(&fingerprint(&[1; 32])).unwrap());
        let reloaded = TrustedPeers::load(&dir).unwrap();
        assert!(!reloaded.is_trusted(&[1; 32]));
        assert!(
            !reloaded.is_trusted(&[2; 32]),
            "introduced by the forgotten PC"
        );
        assert!(reloaded.is_trusted(&[3; 32]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_line_is_an_error() {
        let dir = temp_dir("damaged");
        fs::write(dir.join(FILE_NAME), "not-a-key somewhere\n").unwrap();
        let err = TrustedPeers::load(&dir).err().unwrap();
        assert!(err.contains("line 1 is damaged"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refused_address_waits_then_may_ask_again() {
        let ip: std::net::IpAddr = "192.0.2.9".parse().unwrap();
        let other: std::net::IpAddr = "192.0.2.10".parse().unwrap();
        let then = std::time::Instant::now();
        let mut refused = vec![(ip, then)];
        assert!(refused_recently(
            &mut refused,
            ip,
            then + std::time::Duration::from_secs(5)
        ));
        assert!(!refused_recently(&mut refused, other, then));
        assert!(!refused_recently(&mut refused, ip, then + REFUSED_WAIT));
        assert!(refused.is_empty(), "old refusals are forgotten");
    }

    #[test]
    fn pairing_is_open_on_a_fresh_install_and_when_opened() {
        let dir = temp_dir("open");
        let trusted = std::sync::Mutex::new(TrustedPeers::load(&dir).unwrap());
        assert!(
            pairing_open(&trusted),
            "a fresh install pairs with no extra step"
        );
        trusted.lock().unwrap().add([1; 32], "").unwrap();
        *OPEN_UNTIL.lock().unwrap() = None;
        assert!(!pairing_open(&trusted), "closed once a PC is trusted");
        open_pairing();
        assert!(pairing_open(&trusted));
        *OPEN_UNTIL.lock().unwrap() = Some(std::time::Instant::now());
        assert!(!pairing_open(&trusted), "closed again when the time is up");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hex_round_trips() {
        let key = [0xab; 32];
        assert_eq!(crate::hex::decode_key(&crate::hex::encode(&key)), Some(key));
        assert_eq!(crate::hex::decode_key("zz"), None);
    }
}

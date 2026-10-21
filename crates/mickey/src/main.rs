//! Spikes for M0. On Windows (0b): type "hello" and move the mouse. On
//! Linux (0a): grab a keyboard and give it back on the hotkey or a timeout.

mod platform;

#[cfg(windows)]
fn main() {
    if let Err(err) = demo() {
        eprintln!("mickey: input injection failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(err) = grab_demo() {
        eprintln!("mickey: {err}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn grab_demo() -> std::io::Result<()> {
    use platform::linux::Device;
    use std::io;
    use std::path::PathBuf;
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    const EV_KEY: u16 = 1;
    const KEY_RIGHTSHIFT: u16 = 54;
    const KEY_RIGHTALT: u16 = 100; // AltGr
    const GRAB_LIMIT: Duration = Duration::from_secs(10);

    let path: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or_else(|| io::Error::other("usage: mickey /dev/input/eventN"))?
        .into();
    let device = Device::open(&path)
        .map_err(|err| io::Error::other(format!("cannot open {}: {err}", path.display())))?;

    // Grabbing while a key is held leaves it stuck down on the desktop.
    println!("mickey: waiting until no key is held");
    while device.any_key_down()? {
        sleep(Duration::from_millis(10));
    }

    let grab = device.grab()?;
    println!("mickey: keyboard grabbed for at most 10 s");
    println!("mickey: press and release AltGr + Right Shift to get it back");

    let deadline = Instant::now() + GRAB_LIMIT;
    let (mut alt, mut shift, mut both_held) = (false, false, false);
    let mut key_events = 0u32;
    while Instant::now() < deadline {
        let Some(event) = device.read_event()? else {
            sleep(Duration::from_millis(5));
            continue;
        };
        if event.kind != EV_KEY {
            continue;
        }
        key_events += 1;
        // value: 0 = released, 1 = pressed, 2 = auto-repeat.
        match event.code {
            KEY_RIGHTALT => alt = event.value != 0,
            KEY_RIGHTSHIFT => shift = event.value != 0,
            _ => {}
        }
        both_held |= alt && shift;
        if both_held && !alt && !shift {
            println!("mickey: hotkey released");
            break;
        }
    }

    drop(grab);
    println!("mickey: keyboard released ({key_events} key events seen)");
    Ok(())
}

#[cfg(windows)]
fn demo() -> std::io::Result<()> {
    use platform::windows::{move_mouse, send_key};
    use std::thread::sleep;
    use std::time::Duration;

    const SCAN_H: u16 = 0x23;
    const SCAN_E: u16 = 0x12;
    const SCAN_L: u16 = 0x26;
    const SCAN_O: u16 = 0x18;

    println!("mickey: click on a text window now, starting in 8 seconds");
    sleep(Duration::from_secs(8));

    for (dx, dy) in [(10, 0), (0, 10), (-10, 0), (0, -10)] {
        for _ in 0..50 {
            move_mouse(dx, dy)?;
            sleep(Duration::from_millis(20));
        }
    }

    for scan_code in [SCAN_H, SCAN_E, SCAN_L, SCAN_L, SCAN_O] {
        // Release even if the press failed, so no key stays held.
        let pressed = send_key(scan_code, true);
        let released = send_key(scan_code, false);
        pressed?;
        released?;
        sleep(Duration::from_millis(50));
    }

    println!("mickey: done");
    Ok(())
}

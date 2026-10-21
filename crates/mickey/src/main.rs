//! Spike 0b: prove that Mickey can type and move the mouse on Windows.
//!
//! Waits eight seconds, draws a small square with the cursor and types
//! "hello" into whatever window has focus. Step 1 of M0 replaces this with
//! the real `source`/`target` CLI.

mod platform;

#[cfg(windows)]
fn main() {
    if let Err(err) = demo() {
        eprintln!("mickey: input injection failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("mickey: spike 0b only runs on Windows");
    std::process::exit(1);
}

#[cfg(windows)]
fn demo() -> std::io::Result<()> {
    use platform::windows::{move_mouse, send_key};
    use std::thread::sleep;
    use std::time::Duration;

    // Set 1 scan codes (the physical key positions on a US keyboard).
    const SCAN_H: u16 = 0x23;
    const SCAN_E: u16 = 0x12;
    const SCAN_L: u16 = 0x26;
    const SCAN_O: u16 = 0x18;

    println!("mickey: click on a text window now, starting in 8 seconds");
    sleep(Duration::from_secs(8));

    // A 500-unit square in 50 small steps per side, slow enough to see.
    for (dx, dy) in [(10, 0), (0, 10), (-10, 0), (0, -10)] {
        for _ in 0..50 {
            move_mouse(dx, dy)?;
            sleep(Duration::from_millis(20));
        }
    }

    for scan_code in [SCAN_H, SCAN_E, SCAN_L, SCAN_L, SCAN_O] {
        // Always try the release, even if the press failed, so no key is
        // ever left held down on Windows.
        let pressed = send_key(scan_code, true);
        let released = send_key(scan_code, false);
        pressed?;
        released?;
        sleep(Duration::from_millis(50));
    }

    println!("mickey: done");
    Ok(())
}

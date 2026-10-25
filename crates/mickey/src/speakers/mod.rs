//! Taking this PC's sound off its speakers while Mickey sends it to the PC
//! that listens (PRD 0012, *Send only*), and always giving it back: when
//! sending stops (`Diverted` dropped), at start (`clean_up`, for a Mickey
//! that ended while sending), and on a stop signal on Linux.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{clean_up, divert};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{clean_up, divert};

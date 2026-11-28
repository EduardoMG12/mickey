//! macOS shared audio playback via CoreAudio's `AudioQueue` (`AudioToolbox`)
//! for Apple Silicon (`aarch64-apple-darwin`).
//!
//! `AudioQueuePlayer` accepts Mickey's exact 48 kHz stereo 16-bit signed LE
//! PCM frames with 4 x 5 ms queued buffers and blocking backpressure so the
//! jitter buffer stays paced.

use crate::audio::wire::{CHANNELS, PCM_BYTES, RATE};
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
const K_PCM_FLAG_IS_SIGNED_INT: u32 = 1 << 2;
const K_PCM_FLAG_IS_PACKED: u32 = 1 << 3;

/// 4 x 5 ms queued buffers; total playback latency including AudioQueue and
/// device buffering is not yet measured.
const PLAY_BUFFERS: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

const _: () = assert!(size_of::<AudioStreamBasicDescription>() == 40);

#[repr(C)]
struct AudioQueueBuffer {
    audio_data_bytes_capacity: u32,
    audio_data: *mut c_void,
    audio_data_byte_size: u32,
    user_data: *mut c_void,
    packet_description_capacity: u32,
    packet_descriptions: *mut c_void,
    packet_description_count: u32,
}

const _: () = assert!(size_of::<AudioQueueBuffer>() == 56);

type AudioQueueOutputCallback =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut AudioQueueBuffer);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(
        format: *const AudioStreamBasicDescription,
        callback: AudioQueueOutputCallback,
        user_data: *mut c_void,
        callback_run_loop: *mut c_void,
        callback_run_loop_mode: *const c_void,
        flags: u32,
        out_queue: *mut *mut c_void,
    ) -> i32;
    fn AudioQueueAllocateBuffer(
        queue: *mut c_void,
        buffer_byte_size: u32,
        out_buffer: *mut *mut AudioQueueBuffer,
    ) -> i32;
    fn AudioQueueEnqueueBuffer(
        queue: *mut c_void,
        buffer: *mut AudioQueueBuffer,
        num_packet_descs: u32,
        packet_descs: *const c_void,
    ) -> i32;
    fn AudioQueueStart(queue: *mut c_void, start_time: *const c_void) -> i32;
    fn AudioQueueStop(queue: *mut c_void, immediate: u8) -> i32;
    fn AudioQueueDispose(queue: *mut c_void, immediate: u8) -> i32;
}

/// Wrapper so raw `AudioQueueBuffer` pointers can cross the free-buffer channel.
struct BufferPtr(*mut AudioQueueBuffer);
// SAFETY: `AudioQueueBuffer` pointers are owned by the `AudioQueue` and only
// mutated by `AudioQueuePlayer::write` while dequeued from CoreAudio.
unsafe impl Send for BufferPtr {}

/// CoreAudio `AudioQueue` playback stream (48 kHz, stereo, 16-bit LE PCM).
pub struct AudioQueuePlayer {
    queue: *mut c_void,
    free_rx: Receiver<BufferPtr>,
    /// Boxed `Sender<BufferPtr>` passed as `user_data` to `AudioQueueNewOutput`,
    /// freed in `Drop` after `AudioQueueDispose` guarantees no more callbacks.
    callback_ctx: *mut Sender<BufferPtr>,
    /// Number of initial buffers queued before starting playback (`0..=PLAY_BUFFERS`).
    primed: usize,
}

unsafe extern "C" fn queue_output_callback(
    user_data: *mut c_void,
    _queue: *mut c_void,
    buffer: *mut AudioQueueBuffer,
) {
    if user_data.is_null() || buffer.is_null() {
        return;
    }
    // SAFETY: `user_data` is the `Sender<BufferPtr>` allocated in
    // `AudioQueuePlayer::open` and kept alive until after `AudioQueueDispose`.
    let free_tx = unsafe { &*(user_data.cast::<Sender<BufferPtr>>()) };
    let _ = free_tx.send(BufferPtr(buffer));
}

impl AudioQueuePlayer {
    pub fn open(pc: &str) -> Result<Self, String> {
        let format = AudioStreamBasicDescription {
            sample_rate: f64::from(RATE),
            format_id: K_AUDIO_FORMAT_LINEAR_PCM,
            format_flags: K_PCM_FLAG_IS_SIGNED_INT | K_PCM_FLAG_IS_PACKED,
            bytes_per_packet: (CHANNELS * 2) as u32,
            frames_per_packet: 1,
            bytes_per_frame: (CHANNELS * 2) as u32,
            channels_per_frame: CHANNELS as u32,
            bits_per_channel: 16,
            reserved: 0,
        };
        let (free_tx, free_rx) = mpsc::channel();
        let callback_ctx = Box::into_raw(Box::new(free_tx.clone()));
        let mut queue: *mut c_void = std::ptr::null_mut();
        // SAFETY: `format` is valid; `callback_ctx` outlives `queue` because
        // `Drop` disposes `queue` synchronously before freeing `callback_ctx`.
        let status = unsafe {
            AudioQueueNewOutput(
                &format,
                queue_output_callback,
                callback_ctx.cast(),
                std::ptr::null_mut(),
                std::ptr::null(),
                0,
                &mut queue,
            )
        };
        if status != 0 || queue.is_null() {
            // SAFETY: queue creation failed, so no callback can ever fire.
            drop(unsafe { Box::from_raw(callback_ctx) });
            return Err(format!(
                "cannot play {pc}'s sound: AudioQueueNewOutput failed ({status})"
            ));
        }

        for _ in 0..PLAY_BUFFERS {
            let mut buf: *mut AudioQueueBuffer = std::ptr::null_mut();
            // SAFETY: `queue` is a valid `AudioQueueRef`.
            let status = unsafe { AudioQueueAllocateBuffer(queue, PCM_BYTES as u32, &mut buf) };
            if status != 0 || buf.is_null() {
                // SAFETY: `queue` is disposed synchronously before freeing `callback_ctx`.
                unsafe {
                    AudioQueueDispose(queue, 1);
                    drop(Box::from_raw(callback_ctx));
                }
                return Err(format!(
                    "cannot play {pc}'s sound: AudioQueueAllocateBuffer failed ({status})"
                ));
            }
            let _ = free_tx.send(BufferPtr(buf));
        }

        Ok(Self {
            queue,
            free_rx,
            callback_ctx,
            primed: 0,
        })
    }

    pub fn write(&mut self, pcm: &[u8; PCM_BYTES]) -> Result<(), String> {
        let BufferPtr(buf) = self
            .free_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| "CoreAudio playback stalled waiting for a buffer".to_string())?;
        // SAFETY: `buf` was allocated with capacity `PCM_BYTES` and is currently
        // owned by us (dequeued from `AudioQueue`).
        unsafe {
            let dst = (*buf).audio_data.cast::<u8>();
            let cap = (*buf).audio_data_bytes_capacity as usize;
            if dst.is_null() || cap < PCM_BYTES {
                return Err("CoreAudio buffer too small".into());
            }
            std::ptr::copy_nonoverlapping(pcm.as_ptr(), dst, PCM_BYTES);
            (*buf).audio_data_byte_size = PCM_BYTES as u32;
            let status = AudioQueueEnqueueBuffer(self.queue, buf, 0, std::ptr::null());
            if status != 0 {
                return Err(format!("AudioQueueEnqueueBuffer failed ({status})"));
            }
            if self.primed < PLAY_BUFFERS {
                if self.primed + 1 == PLAY_BUFFERS {
                    let status = AudioQueueStart(self.queue, std::ptr::null());
                    if status != 0 {
                        return Err(format!("AudioQueueStart failed ({status})"));
                    }
                }
                self.primed += 1;
            }
        }
        Ok(())
    }
}

impl Drop for AudioQueuePlayer {
    fn drop(&mut self) {
        // SAFETY: synchronous stop + dispose guarantees CoreAudio invokes no
        // further callbacks before we reclaim `callback_ctx`.
        unsafe {
            AudioQueueStop(self.queue, 1);
            AudioQueueDispose(self.queue, 1);
            drop(Box::from_raw(self.callback_ctx));
        }
    }
}

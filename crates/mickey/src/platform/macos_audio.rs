//! macOS shared audio via CoreAudio (`AudioToolbox` + `CoreAudio` Process Taps)
//! for Apple Silicon (`aarch64-apple-darwin`, macOS 14.2+).
//!
//! - **Playback (`AudioQueuePlayer`)**: `AudioQueue` accepting exact 48 kHz
//!   stereo 16-bit signed LE PCM with 4 x 5 ms queued buffers and blocking
//!   backpressure so Mickey's jitter buffer stays paced.
//! - **Capture (`TapRecorder`)**: `AudioHardwareCreateProcessTap` attached to
//!   a private aggregate device, with `CATapMuted` when diverting speakers or
//!   `CATapUnmuted` when keeping local sound. No third-party virtual driver is
//!   needed, and CoreAudio restores system audio automatically when the tap is
//!   destroyed or the process exits.

use crate::audio::wire::{CHANNELS, PCM_BYTES, RATE};
use std::ffi::{CStr, CString, c_char, c_void};
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::time::Duration;

const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
const K_PCM_FLAG_IS_FLOAT: u32 = 1 << 0;
const K_PCM_FLAG_IS_SIGNED_INT: u32 = 1 << 2;
const K_PCM_FLAG_IS_PACKED: u32 = 1 << 3;
const K_PCM_FLAG_IS_NON_INTERLEAVED: u32 = 1 << 5;

const K_AUDIO_TAP_PROPERTY_UID: u32 = u32::from_be_bytes(*b"tuid");
const K_AUDIO_TAP_PROPERTY_FORMAT: u32 = u32::from_be_bytes(*b"tfmt");
const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;

/// 4 x 5 ms queued buffers; total playback latency including AudioQueue and
/// device buffering is not yet measured.
const PLAY_BUFFERS: usize = 4;
/// Bounded capture queue (40 x 5 ms = 200 ms) so the real-time CoreAudio
/// callback never blocks or allocates.
const CAPTURE_QUEUE_FRAMES: usize = 40;
/// `CATapUnmuted = 0`, `CATapMuted = 1`.
const CA_TAP_UNMUTED: isize = 0;
const CA_TAP_MUTED: isize = 1;

static NEXT_AGGREGATE_SEQ: AtomicU64 = AtomicU64::new(1);

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

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

const _: () = assert!(size_of::<AudioBuffer>() == 16);

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

const _: () = assert!(size_of::<AudioBufferList>() == 24);

type AudioQueueOutputCallback =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut AudioQueueBuffer);
type AudioDeviceIOProc = unsafe extern "C" fn(
    u32,
    *const c_void,
    *const AudioBufferList,
    *const c_void,
    *mut AudioBufferList,
    *const c_void,
    *mut c_void,
) -> i32;

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

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioHardwareCreateProcessTap(tap_description: *mut c_void, out_tap_id: *mut u32) -> i32;
    fn AudioHardwareDestroyProcessTap(tap_id: u32) -> i32;
    fn AudioHardwareCreateAggregateDevice(
        description: *const c_void,
        out_device_id: *mut u32,
    ) -> i32;
    fn AudioHardwareDestroyAggregateDevice(device_id: u32) -> i32;
    fn AudioObjectGetPropertyData(
        object_id: u32,
        address: *const AudioObjectPropertyAddress,
        qualifier_data_size: u32,
        qualifier_data: *const c_void,
        io_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> i32;
    fn AudioDeviceCreateIOProcID(
        device_id: u32,
        proc: AudioDeviceIOProc,
        client_data: *mut c_void,
        out_proc_id: *mut *mut c_void,
    ) -> i32;
    fn AudioDeviceDestroyIOProcID(device_id: u32, proc_id: *mut c_void) -> i32;
    fn AudioDeviceStart(device_id: u32, proc_id: *mut c_void) -> i32;
    fn AudioDeviceStop(device_id: u32, proc_id: *mut c_void) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
    fn objc_msgSend();
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

/// Real-time capture state owned by `TapRecorder` and accessed inside `io_proc`.
struct CaptureState {
    tx: SyncSender<[u8; PCM_BYTES]>,
    format: AudioStreamBasicDescription,
    /// Pending 5 ms output frame being filled.
    out: [u8; PCM_BYTES],
    out_pos: usize,
    /// Fractional source-frame accumulator for linear resampling when the tap's
    /// native sample rate differs from 48 kHz.
    resample_phase: f64,
}

impl CaptureState {
    fn push_stereo(&mut self, left: f32, right: f32) {
        let l = (left.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let r = (right.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        self.out[self.out_pos..self.out_pos + 2].copy_from_slice(&l.to_le_bytes());
        self.out[self.out_pos + 2..self.out_pos + 4].copy_from_slice(&r.to_le_bytes());
        self.out_pos += 4;
        if self.out_pos == PCM_BYTES {
            let _ = self.tx.try_send(self.out);
            self.out_pos = 0;
        }
    }

    fn feed_f32_frames(&mut self, frames: &[(f32, f32)]) {
        let in_rate = self.format.sample_rate;
        if frames.is_empty() || !in_rate.is_finite() || in_rate <= 0.0 {
            return;
        }
        let out_rate = f64::from(RATE);
        if (in_rate - out_rate).abs() < 1.0 {
            for &(l, r) in frames {
                self.push_stereo(l, r);
            }
            return;
        }
        let step = in_rate / out_rate;
        let len = frames.len() as f64;
        if self.resample_phase < 0.0 {
            self.resample_phase = 0.0;
        }
        while self.resample_phase < len {
            let i0 = (self.resample_phase as usize).min(frames.len() - 1);
            let frac = (self.resample_phase - i0 as f64) as f32;
            let (l0, r0) = frames[i0];
            let (l1, r1) = frames[(i0 + 1).min(frames.len() - 1)];
            self.push_stereo(l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac);
            self.resample_phase += step;
        }
        self.resample_phase -= len;
    }
}

unsafe extern "C" fn tap_io_proc(
    _device_id: u32,
    _now: *const c_void,
    input_data: *const AudioBufferList,
    _input_time: *const c_void,
    _output_data: *mut AudioBufferList,
    _output_time: *const c_void,
    client_data: *mut c_void,
) -> i32 {
    if input_data.is_null() || client_data.is_null() {
        return 0;
    }
    // SAFETY: `client_data` is the `CaptureState` owned by `TapRecorder`,
    // exclusively accessed by CoreAudio's single serial IOProc thread until
    // `AudioDeviceDestroyIOProcID` completes in `Drop`.
    let state = unsafe { &mut *(client_data.cast::<CaptureState>()) };
    // SAFETY: `input_data` is a valid `AudioBufferList` provided by CoreAudio.
    let n_bufs = unsafe { (*input_data).number_buffers as usize };
    if n_bufs == 0 {
        return 0;
    }
    // SAFETY: `buffers` points to `n_bufs` contiguous `AudioBuffer` structs.
    let buffers = unsafe { std::slice::from_raw_parts((*input_data).buffers.as_ptr(), n_bufs) };
    let is_f32 =
        state.format.format_flags & K_PCM_FLAG_IS_FLOAT != 0 && state.format.bits_per_channel == 32;
    let non_interleaved = state.format.format_flags & K_PCM_FLAG_IS_NON_INTERLEAVED != 0;
    if !is_f32 {
        return 0;
    }

    // Stack-allocated chunk buffer so the real-time callback never allocates.
    let mut chunk = [(0.0f32, 0.0f32); 256];
    if non_interleaved && n_bufs >= 2 {
        let count_l = buffers[0].data_byte_size as usize / size_of::<f32>();
        let count_r = buffers[1].data_byte_size as usize / size_of::<f32>();
        let total = count_l.min(count_r);
        let ptr_l = buffers[0].data.cast::<f32>();
        let ptr_r = buffers[1].data.cast::<f32>();
        if ptr_l.is_null() || ptr_r.is_null() || !ptr_l.is_aligned() || !ptr_r.is_aligned() {
            return 0;
        }
        // SAFETY: CoreAudio guarantees `data` holds `data_byte_size` bytes of aligned f32.
        let left = unsafe { std::slice::from_raw_parts(ptr_l, total) };
        let right = unsafe { std::slice::from_raw_parts(ptr_r, total) };
        let mut offset = 0;
        while offset < total {
            let take = (total - offset).min(chunk.len());
            for (i, slot) in chunk[..take].iter_mut().enumerate() {
                *slot = (left[offset + i], right[offset + i]);
            }
            state.feed_f32_frames(&chunk[..take]);
            offset += take;
        }
    } else if !buffers[0].data.is_null() {
        let ptr = buffers[0].data.cast::<f32>();
        if !ptr.is_aligned() {
            return 0;
        }
        let channels = (buffers[0].number_channels as usize).max(1);
        let samples = buffers[0].data_byte_size as usize / size_of::<f32>();
        let total = samples / channels;
        // SAFETY: CoreAudio guarantees `data` holds `data_byte_size` bytes of aligned f32.
        let data = unsafe { std::slice::from_raw_parts(ptr, samples) };
        let mut offset = 0;
        while offset < total {
            let take = (total - offset).min(chunk.len());
            for (i, slot) in chunk[..take].iter_mut().enumerate() {
                let base = (offset + i) * channels;
                let l = data[base];
                let r = if channels >= 2 { data[base + 1] } else { l };
                *slot = (l, r);
            }
            state.feed_f32_frames(&chunk[..take]);
            offset += take;
        }
    }
    0
}

/// Core Audio Process Tap system-audio recorder.
pub struct TapRecorder {
    tap_id: u32,
    aggregate_id: u32,
    proc_id: *mut c_void,
    rx: Receiver<[u8; PCM_BYTES]>,
    state: *mut CaptureState,
}

impl TapRecorder {
    pub fn open(diverted: bool) -> Result<Self, String> {
        let tap_id = create_process_tap(diverted)?;
        let result = Self::attach_aggregate(tap_id);
        if result.is_err() {
            // SAFETY: `tap_id` was created above and must not leak on error.
            unsafe { AudioHardwareDestroyProcessTap(tap_id) };
        }
        result
    }

    fn attach_aggregate(tap_id: u32) -> Result<Self, String> {
        let format = tap_format(tap_id)?;
        if !format.sample_rate.is_finite() || format.sample_rate <= 0.0 {
            return Err(format!(
                "invalid Process Tap sample rate ({})",
                format.sample_rate
            ));
        }
        let tap_uid = tap_uid_cfstring(tap_id)?;
        let aggregate_id = create_aggregate_for_tap(tap_uid);
        // SAFETY: `tap_uid` is a retained `CFStringRef` returned by CoreAudio.
        unsafe { CFRelease(tap_uid.cast()) };
        let aggregate_id = aggregate_id?;

        let (tx, rx) = mpsc::sync_channel(CAPTURE_QUEUE_FRAMES);
        let state = Box::into_raw(Box::new(CaptureState {
            tx,
            format,
            out: [0u8; PCM_BYTES],
            out_pos: 0,
            resample_phase: 0.0,
        }));
        let mut proc_id: *mut c_void = std::ptr::null_mut();
        // SAFETY: `aggregate_id` is valid; `state` stays alive until after
        // `AudioDeviceDestroyIOProcID` in `Drop`.
        let status = unsafe {
            AudioDeviceCreateIOProcID(aggregate_id, tap_io_proc, state.cast(), &mut proc_id)
        };
        if status != 0 || proc_id.is_null() {
            // SAFETY: IOProc was not created; destroy aggregate and reclaim state.
            unsafe {
                AudioHardwareDestroyAggregateDevice(aggregate_id);
                drop(Box::from_raw(state));
            }
            return Err(format!("AudioDeviceCreateIOProcID failed ({status})"));
        }
        // SAFETY: `aggregate_id` and `proc_id` are valid.
        let status = unsafe { AudioDeviceStart(aggregate_id, proc_id) };
        if status != 0 {
            // SAFETY: stop/destroy IOProc and aggregate before reclaiming state.
            unsafe {
                AudioDeviceDestroyIOProcID(aggregate_id, proc_id);
                AudioHardwareDestroyAggregateDevice(aggregate_id);
                drop(Box::from_raw(state));
            }
            return Err(format!("AudioDeviceStart failed ({status})"));
        }

        Ok(Self {
            tap_id,
            aggregate_id,
            proc_id,
            rx,
            state,
        })
    }

    /// Reads the next 5 ms frame, returning `Ok(None)` on a short timeout so
    /// the caller can check its stop flag promptly.
    pub fn read(&mut self) -> Result<Option<[u8; PCM_BYTES]>, String> {
        match self.rx.recv_timeout(Duration::from_millis(50)) {
            Ok(pcm) => Ok(Some(pcm)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("CoreAudio capture stopped unexpectedly".into())
            }
        }
    }
}

impl Drop for TapRecorder {
    fn drop(&mut self) {
        // SAFETY: stop and destroy the IOProc first (blocks until any active
        // callback returns), then destroy aggregate device and Process Tap
        // (automatically unmuting system audio), then reclaim `CaptureState`.
        unsafe {
            AudioDeviceStop(self.aggregate_id, self.proc_id);
            AudioDeviceDestroyIOProcID(self.aggregate_id, self.proc_id);
            AudioHardwareDestroyAggregateDevice(self.aggregate_id);
            AudioHardwareDestroyProcessTap(self.tap_id);
            drop(Box::from_raw(self.state));
        }
    }
}

unsafe fn objc_cls(name: &CStr) -> *mut c_void {
    // SAFETY: `name` is a valid NUL-terminated C string.
    unsafe { objc_getClass(name.as_ptr()) }
}

unsafe fn objc_sel(name: &CStr) -> *mut c_void {
    // SAFETY: `name` is a valid NUL-terminated C string.
    unsafe { sel_registerName(name.as_ptr()) }
}

unsafe fn msg_id(obj: *mut c_void, sel: *mut c_void) -> *mut c_void {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    // SAFETY: caller guarantees signature `() -> id`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, sel)
    }
}

unsafe fn msg_id_id(obj: *mut c_void, sel: *mut c_void, arg: *mut c_void) -> *mut c_void {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
    // SAFETY: caller guarantees signature `(id) -> id`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, sel, arg)
    }
}

unsafe fn msg_void_id(obj: *mut c_void, sel: *mut c_void, arg: *mut c_void) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
    // SAFETY: caller guarantees signature `(id) -> void`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, sel, arg);
    }
}

unsafe fn msg_void_bool(obj: *mut c_void, sel: *mut c_void, arg: bool) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, bool);
    // SAFETY: caller guarantees signature `(BOOL) -> void`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, sel, arg);
    }
}

unsafe fn msg_void_isize(obj: *mut c_void, sel: *mut c_void, arg: isize) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, isize);
    // SAFETY: caller guarantees signature `(NSInteger) -> void`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(obj, sel, arg);
    }
}

unsafe fn ns_str(text: &str) -> *mut c_void {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char) -> *mut c_void;
    let cstr = CString::new(text).unwrap_or_default();
    // SAFETY: `NSString` responds to `stringWithUTF8String:`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(
            objc_cls(c"NSString"),
            objc_sel(c"stringWithUTF8String:"),
            cstr.as_ptr(),
        )
    }
}

unsafe fn ns_bool(value: bool) -> *mut c_void {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, bool) -> *mut c_void;
    // SAFETY: `NSNumber` responds to `numberWithBool:`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(objc_cls(c"NSNumber"), objc_sel(c"numberWithBool:"), value)
    }
}

unsafe fn dict_set(dict: *mut c_void, key: &str, value: *mut c_void) {
    type F = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void);
    // SAFETY: `dict` is an `NSMutableDictionary`.
    unsafe {
        let f: F = std::mem::transmute(objc_msgSend as *const ());
        f(dict, objc_sel(c"setObject:forKey:"), value, ns_str(key));
    }
}

fn create_process_tap(diverted: bool) -> Result<u32, String> {
    // SAFETY: constructs a `CATapDescription` via CoreAudio's Objective-C API
    // inside an autorelease pool and passes it to `AudioHardwareCreateProcessTap`.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let tap_cls = objc_cls(c"CATapDescription");
        if tap_cls.is_null() {
            objc_autoreleasePoolPop(pool);
            return Err(
                "CoreAudio Process Taps (CATapDescription) require macOS 14.2 or newer".into(),
            );
        }
        let empty = msg_id(objc_cls(c"NSArray"), objc_sel(c"array"));
        let alloc = msg_id(tap_cls, objc_sel(c"alloc"));
        let desc = msg_id_id(
            alloc,
            objc_sel(c"initStereoGlobalTapButExcludeProcesses:"),
            empty,
        );
        if desc.is_null() {
            objc_autoreleasePoolPop(pool);
            return Err("failed to initialize CATapDescription".into());
        }
        msg_void_id(desc, objc_sel(c"setName:"), ns_str("Mickey"));
        msg_void_bool(desc, objc_sel(c"setPrivateTap:"), true);
        msg_void_isize(
            desc,
            objc_sel(c"setMuteBehavior:"),
            if diverted {
                CA_TAP_MUTED
            } else {
                CA_TAP_UNMUTED
            },
        );
        let mut tap_id = 0u32;
        let status = AudioHardwareCreateProcessTap(desc, &mut tap_id);
        type ReleaseFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
        let release: ReleaseFn = std::mem::transmute(objc_msgSend as *const ());
        release(desc, objc_sel(c"release"));
        objc_autoreleasePoolPop(pool);
        if status != 0 || tap_id == 0 {
            return Err(format!(
                "AudioHardwareCreateProcessTap failed ({status}); grant System Audio Recording \
                 permission in System Settings > Privacy & Security > Screen & System Audio Recording"
            ));
        }
        Ok(tap_id)
    }
}

fn tap_format(tap_id: u32) -> Result<AudioStreamBasicDescription, String> {
    let addr = AudioObjectPropertyAddress {
        selector: K_AUDIO_TAP_PROPERTY_FORMAT,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut format = AudioStreamBasicDescription::default();
    let mut size = size_of::<AudioStreamBasicDescription>() as u32;
    // SAFETY: `format` is a writable `AudioStreamBasicDescription` of `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap_id,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            (&raw mut format).cast(),
        )
    };
    if status != 0 {
        return Err(format!("cannot read Process Tap format ({status})"));
    }
    Ok(format)
}

fn tap_uid_cfstring(tap_id: u32) -> Result<*mut c_void, String> {
    let addr = AudioObjectPropertyAddress {
        selector: K_AUDIO_TAP_PROPERTY_UID,
        scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut uid: *mut c_void = std::ptr::null_mut();
    let mut size = size_of::<*mut c_void>() as u32;
    // SAFETY: `uid` receives a retained `CFStringRef` on success.
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap_id,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            (&raw mut uid).cast(),
        )
    };
    if status != 0 || uid.is_null() {
        return Err(format!("cannot read Process Tap UID ({status})"));
    }
    Ok(uid)
}

fn create_aggregate_for_tap(tap_uid: *mut c_void) -> Result<u32, String> {
    let seq = NEXT_AGGREGATE_SEQ.fetch_add(1, Ordering::Relaxed);
    // SAFETY: builds an `NSMutableDictionary` describing a private aggregate
    // device containing our Process Tap and passes it to CoreAudio.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let sub_tap = msg_id(objc_cls(c"NSMutableDictionary"), objc_sel(c"dictionary"));
        if sub_tap.is_null() {
            objc_autoreleasePoolPop(pool);
            return Err("cannot allocate aggregate sub-tap dictionary".into());
        }
        dict_set(sub_tap, "uid", tap_uid);
        dict_set(sub_tap, "drift", ns_bool(true));
        let tap_list = msg_id_id(objc_cls(c"NSArray"), objc_sel(c"arrayWithObject:"), sub_tap);

        let desc = msg_id(objc_cls(c"NSMutableDictionary"), objc_sel(c"dictionary"));
        if tap_list.is_null() || desc.is_null() {
            objc_autoreleasePoolPop(pool);
            return Err("cannot allocate aggregate device description".into());
        }
        dict_set(desc, "name", ns_str("Mickey Tap Aggregate"));
        dict_set(
            desc,
            "uid",
            ns_str(&format!(
                "io.github.eduardomg12.mickey.tap.{}.{seq}",
                std::process::id()
            )),
        );
        dict_set(desc, "private", ns_bool(true));
        dict_set(desc, "stacked", ns_bool(false));
        dict_set(desc, "tapautostart", ns_bool(true));
        dict_set(desc, "taplist", tap_list);

        let mut agg_id = 0u32;
        let status = AudioHardwareCreateAggregateDevice(desc, &mut agg_id);
        objc_autoreleasePoolPop(pool);
        if status != 0 || agg_id == 0 {
            return Err(format!(
                "AudioHardwareCreateAggregateDevice failed ({status})"
            ));
        }
        Ok(agg_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEREO_FRAMES_PER_PACKET: usize = PCM_BYTES / (CHANNELS * 2);

    fn capture_state(sample_rate: f64) -> (CaptureState, Receiver<[u8; PCM_BYTES]>) {
        let (tx, rx) = mpsc::sync_channel(8);
        let state = CaptureState {
            tx,
            format: AudioStreamBasicDescription {
                sample_rate,
                ..Default::default()
            },
            out: [0u8; PCM_BYTES],
            out_pos: 0,
            resample_phase: 0.0,
        };
        (state, rx)
    }

    #[test]
    fn capture_state_packs_48khz_and_clamps_out_of_range_samples() {
        let (mut state, rx) = capture_state(48_000.0);
        let frames = vec![(1.5f32, -1.5f32); STEREO_FRAMES_PER_PACKET];
        state.feed_f32_frames(&frames);
        let pcm = rx.try_recv().expect("one 5 ms frame emitted");
        let l = i16::from_le_bytes([pcm[0], pcm[1]]);
        let r = i16::from_le_bytes([pcm[2], pcm[3]]);
        assert_eq!(l, i16::MAX);
        assert_eq!(r, -i16::MAX);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn capture_state_resamples_44100hz_to_48khz_frames() {
        let (mut state, rx) = capture_state(44_100.0);
        // 441 source frames = 10 ms -> should emit 2 full 5 ms output frames (480 samples).
        let frames = vec![(0.5f32, -0.5f32); 441];
        state.feed_f32_frames(&frames);
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn capture_state_ignores_non_positive_sample_rate() {
        let (mut state, rx) = capture_state(0.0);
        state.feed_f32_frames(&[(0.25, 0.25); STEREO_FRAMES_PER_PACKET]);
        assert!(rx.try_recv().is_err());
    }
}

//! Microphone capture via cpal → PCM16/16k mono. A dedicated thread owns the
//! (non-Send) cpal stream; capture only accumulates while `recording` is true.
//! On stop, the buffer is framed into AudioFrame(0x01) messages and sent over
//! the WS channel (same wire format the firmware uses for audio_input).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::ws::WsOut;

const TARGET_RATE: f32 = 16000.0;
const VAD_SPEAK_RMS: f64 = 0.02; // above this = speech
const VAD_SILENCE_SECS: f32 = 3.0; // trailing silence to auto-stop

pub struct Recorder {
    recording: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<i16>>>,
    done: Arc<AtomicBool>, // VAD: speech ended (trailing silence exceeded)
    vad: Arc<Mutex<(bool, usize)>>, // (heard speech, trailing silence samples)
}

impl Recorder {
    /// Spawn the capture thread (opens the default input device) and return a
    /// handle. The thread keeps the stream alive; `recording` gates capture.
    pub fn new() -> Self {
        let recording = Arc::new(AtomicBool::new(false));
        let buffer = Arc::new(Mutex::new(Vec::<i16>::new()));
        let done = Arc::new(AtomicBool::new(false));
        let vad = Arc::new(Mutex::new((false, 0usize)));
        std::thread::spawn({
            let rec = recording.clone();
            let buf = buffer.clone();
            let done = done.clone();
            let vad = vad.clone();
            move || capture_thread(rec, buf, done, vad)
        });
        Self { recording, buffer, done, vad }
    }

    pub fn start(&self) {
        self.buffer.lock().unwrap().clear();
        *self.vad.lock().unwrap() = (false, 0);
        self.done.store(false, Ordering::Relaxed);
        self.recording.store(true, Ordering::Relaxed);
    }

    /// VAD detected the user stopped talking (trailing silence exceeded).
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }

    /// Stop and return the captured PCM16/16k samples.
    pub fn stop(&self) -> Vec<i16> {
        self.recording.store(false, Ordering::Relaxed);
        self.done.store(false, Ordering::Relaxed);
        std::mem::take(&mut *self.buffer.lock().unwrap())
    }
}

fn capture_thread(
    recording: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<i16>>>,
    done: Arc<AtomicBool>,
    vad: Arc<Mutex<(bool, usize)>>,
) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let Some(device) = host.default_input_device() else {
        eprintln!("[audio] no input device");
        return;
    };
    let Ok(config) = device.default_input_config() else {
        eprintln!("[audio] no default input config");
        return;
    };
    let src_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    let ratio = (src_rate / TARGET_RATE).max(1.0);
    let err_fn = |e| eprintln!("[audio] stream error: {e}");

    // Only handle f32 input (the common macOS default); decimate to 16k mono.
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            let rec = recording.clone();
            let buf = buffer.clone();
            let done = done.clone();
            let vad = vad.clone();
            let phase = Arc::new(Mutex::new(0.0f32));
            device.build_input_stream(
                &config.into(),
                move |data: &[f32], _| {
                    if !rec.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut out = buf.lock().unwrap();
                    let start_len = out.len();
                    {
                        let mut p = phase.lock().unwrap();
                        let mut i = 0;
                        while i < data.len() {
                            *p -= 1.0;
                            if *p < 0.0 {
                                *p += ratio;
                                let s = data[i]; // channel 0 of this frame
                                out.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                            }
                            i += channels;
                        }
                    }
                    // VAD on the samples just decimated: track trailing silence
                    let pushed = &out[start_len..];
                    if !pushed.is_empty() {
                        let sum: f64 = pushed
                            .iter()
                            .map(|&s| {
                                let f = s as f64 / 32768.0;
                                f * f
                            })
                            .sum();
                        let rms = (sum / pushed.len() as f64).sqrt();
                        let mut v = vad.lock().unwrap();
                        if rms > VAD_SPEAK_RMS {
                            v.0 = true; // heard speech
                            v.1 = 0; // reset trailing silence
                        } else if v.0 {
                            v.1 += pushed.len();
                            if v.1 as f32 / TARGET_RATE > VAD_SILENCE_SECS {
                                done.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                },
                err_fn,
                None,
            )
        }
        other => {
            eprintln!("[audio] unsupported input sample format: {other:?}");
            return;
        }
    };
    let Ok(stream) = stream else {
        eprintln!("[audio] failed to build input stream");
        return;
    };
    if stream.play().is_err() {
        eprintln!("[audio] failed to start input stream");
        return;
    }
    // Keep the stream (and thread) alive for the app's lifetime.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Frame PCM16/16k into AudioFrame(0x01) messages and push them onto the WS
/// outgoing channel. First frame carries START, last carries END (so the server
/// transitions Listening → Thinking and runs STT).
pub fn send_pcm(tx: &mpsc::UnboundedSender<WsOut>, pcm: &[i16]) {
    const FLAG_START: u8 = 0x01;
    const FLAG_END: u8 = 0x02;
    const CHUNK_SAMPLES: usize = 1600; // 100ms @ 16k

    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    let chunk_bytes = CHUNK_SAMPLES * 2;
    let n_chunks = ((bytes.len() + chunk_bytes - 1) / chunk_bytes).max(1);
    for i in 0..n_chunks {
        let start = i * chunk_bytes;
        let end = (start + chunk_bytes).min(bytes.len());
        let payload = if start < bytes.len() { &bytes[start..end] } else { &[][..] };
        let mut flags = 0u8;
        if i == 0 {
            flags |= FLAG_START;
        }
        if i == n_chunks - 1 {
            flags |= FLAG_END;
        }
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.push(0x01); // AUDIO_INPUT
        frame.push(flags);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        frame.extend_from_slice(payload);
        let _ = tx.send(WsOut::Binary(frame));
    }
}

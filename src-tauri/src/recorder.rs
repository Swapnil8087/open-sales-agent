//! Owns the microphone stream and the worker thread while recording.

use crate::audio::{self, Resampler, SAMPLE_RATE};
use crate::engine::{Emit, Engine, Event};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Recorder {
    stop: Arc<AtomicBool>,
    mic_thread: JoinHandle<()>,
    worker: JoinHandle<()>,
    engine: Arc<Engine>,
    emit: Emit,
}

fn open_stream(
    on_chunk: impl Fn(Vec<f32>) + Send + 'static,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let dev = host.default_input_device().ok_or("No microphone found")?;
    let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
    let channels = cfg.channels() as usize;
    let rate = cfg.sample_rate();
    let mut rs = Resampler::new(rate, SAMPLE_RATE);
    let err_fn = |e| eprintln!("audio stream error: {e}");
    let mut handle = move |mono: Vec<f32>| {
        let out = rs.process(&mono);
        if !out.is_empty() {
            on_chunk(out);
        }
    };
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_input_stream(
            cfg.into(),
            move |d: &[f32], _| handle(audio::downmix_to_mono(d, channels)),
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => dev.build_input_stream(
            cfg.into(),
            move |d: &[i16], _| {
                let f: Vec<f32> = d.iter().map(|v| *v as f32 / 32768.0).collect();
                handle(audio::downmix_to_mono(&f, channels))
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => dev.build_input_stream(
            cfg.into(),
            move |d: &[u16], _| {
                let f: Vec<f32> = d.iter().map(|v| (*v as f32 - 32768.0) / 32768.0).collect();
                handle(audio::downmix_to_mono(&f, channels))
            },
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

impl Recorder {
    pub fn start(engine: Arc<Engine>, emit: Emit) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        // cpal streams are !Send, so the stream lives and dies on this thread.
        let mic_thread = {
            let (engine, emit, stop) = (engine.clone(), emit.clone(), stop.clone());
            std::thread::spawn(move || {
                let eng = engine.clone();
                let em = emit.clone();
                let stream = open_stream(move |mono| {
                    em(Event::Level(audio::level(&mono)));
                    eng.push(&audio::f32_to_i16(&mono));
                });
                match stream {
                    Ok(s) => {
                        let _ = ready_tx.send(Ok(()));
                        while !stop.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        drop(s); // closes the mic immediately
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
        };
        ready_rx.recv().map_err(|e| e.to_string())??;

        let worker = {
            let (engine, emit, stop) = (engine.clone(), emit.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    engine.tick(&*emit);
                    std::thread::sleep(Duration::from_millis(400));
                }
            })
        };
        Ok(Self { stop, mic_thread, worker, engine, emit })
    }

    /// Stop recording, run the final pass, return the full transcript.
    pub fn stop(self) -> String {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.mic_thread.join();
        let _ = self.worker.join();
        self.engine.finish(&*self.emit)
    }
}

//! Live transcription engine: buffers audio, schedules whisper passes, stitches text.

use crate::audio::{self, SAMPLE_RATE};
use crate::session::{plan, Step, SR};
use crate::stt::Transcriber;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Level(f32),
    Partial(String),
    Final(String),
    Error(String),
}

pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

const MIN_PEAK: u16 = 200; // ~ -44 dBFS; a muted/denied mic delivers exactly 0
const SILENT: &str = "No sound detected from the microphone. Check System Settings > Privacy & Security > Microphone and your input device.";

#[derive(Default)]
struct State {
    warned_silent: bool,
    seg_start: usize,
    last_attempt: usize,
    committed: String,
}

pub struct Engine {
    samples: Mutex<Vec<i16>>,
    state: Mutex<State>,
    transcriber: Arc<dyn Transcriber>,
    tmp_dir: PathBuf,
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a} {b}"),
    }
}

impl Engine {
    pub fn new(transcriber: Arc<dyn Transcriber>, tmp_dir: PathBuf) -> Self {
        Self { samples: Mutex::new(Vec::new()), state: Mutex::new(State::default()), transcriber, tmp_dir }
    }

    pub fn push(&self, chunk: &[i16]) {
        self.samples.lock().unwrap().extend_from_slice(chunk);
    }

    fn transcribe_range(&self, from: usize, to: usize) -> Result<String, String> {
        let mut seg: Vec<i16> = self.samples.lock().unwrap()[from..to].to_vec();
        // Whisper hallucinates ("you you you") on silence, so don't feed it silence.
        if audio::peak(&seg) < MIN_PEAK {
            return Err(SILENT.to_string());
        }
        audio::normalize(&mut seg); // quiet laptop mics transcribe far better when boosted
        std::fs::create_dir_all(&self.tmp_dir).map_err(|e| e.to_string())?;
        let wav = self.tmp_dir.join("segment.wav");
        audio::write_wav(&wav, &seg)?;
        let res = self.transcriber.transcribe(&wav).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&wav); // never leave client audio behind
        res
    }

    fn tail_silent(&self, total: usize) -> bool {
        let s = self.samples.lock().unwrap();
        let from = total.saturating_sub(SR / 2);
        let tail: Vec<f32> = s[from..total].iter().map(|v| *v as f32 / 32768.0).collect();
        audio::level(&tail) < 0.2
    }

    /// One scheduling step; call periodically while recording.
    pub fn tick(&self, emit: &dyn Fn(Event)) {
        let total = self.samples.lock().unwrap().len();
        let (seg_start, last) = {
            let st = self.state.lock().unwrap();
            (st.seg_start, st.last_attempt)
        };
        let step = plan(total, seg_start, last, self.tail_silent(total));
        if step == Step::Wait {
            return;
        }
        self.state.lock().unwrap().last_attempt = total;
        match self.transcribe_range(seg_start, total) {
            Ok(text) => {
                let mut st = self.state.lock().unwrap();
                st.warned_silent = false;
                if step == Step::Commit {
                    st.committed = join(&st.committed, &text);
                    st.seg_start = total;
                    emit(Event::Partial(st.committed.clone()));
                } else {
                    emit(Event::Partial(join(&st.committed, &text)));
                }
            }
            Err(e) => self.report(e, emit),
        }
    }

    /// Silence is reported once per quiet stretch instead of every tick.
    fn report(&self, e: String, emit: &dyn Fn(Event)) {
        let mut st = self.state.lock().unwrap();
        if e == SILENT {
            if st.warned_silent {
                return;
            }
            st.warned_silent = true;
        }
        drop(st);
        emit(Event::Error(e));
    }

    /// Transcribe whatever is left and return the full text.
    pub fn finish(&self, emit: &dyn Fn(Event)) -> String {
        let total = self.samples.lock().unwrap().len();
        let seg_start = self.state.lock().unwrap().seg_start;
        let mut text = String::new();
        if total.saturating_sub(seg_start) >= SR / 4 {
            match self.transcribe_range(seg_start, total) {
                Ok(t) => text = t,
                Err(e) => self.report(e, emit),
            }
        }
        let full = join(&self.state.lock().unwrap().committed, &text);
        emit(Event::Final(full.clone()));
        full
    }

    pub fn seconds_recorded(&self) -> f32 {
        self.samples.lock().unwrap().len() as f32 / SAMPLE_RATE as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::SttError;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        calls: AtomicUsize,
        lens: Mutex<Vec<usize>>,
        peaks: Mutex<Vec<u16>>,
        fail: bool,
    }
    impl Transcriber for Fake {
        fn transcribe(&self, wav: &Path) -> Result<String, SttError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let mut rd = hound::WavReader::open(wav).unwrap();
            self.lens.lock().unwrap().push(rd.len() as usize);
            let data: Vec<i16> = rd.samples::<i16>().map(|s| s.unwrap()).collect();
            self.peaks.lock().unwrap().push(audio::peak(&data));
            if self.fail {
                return Err(SttError::Failed("boom".into()));
            }
            Ok(format!("t{n}"))
        }
    }

    fn engine(fail: bool) -> (Engine, Arc<Fake>, tempfile::TempDir) {
        let f = Arc::new(Fake { calls: AtomicUsize::new(0), lens: Mutex::new(vec![]), peaks: Mutex::new(vec![]), fail });
        let d = tempfile::tempdir().unwrap();
        (Engine::new(f.clone(), d.path().join("tmp")), f, d)
    }

    fn loud(secs: usize) -> Vec<i16> {
        vec![8000; secs * SR]
    }

    fn collect() -> (Arc<Mutex<Vec<Event>>>, impl Fn(Event)) {
        let ev = Arc::new(Mutex::new(vec![]));
        let e2 = ev.clone();
        (ev, move |e| e2.lock().unwrap().push(e))
    }

    #[test]
    fn emits_partial_after_two_seconds() {
        let (e, f, _d) = engine(false);
        let (ev, emit) = collect();
        e.push(&loud(1));
        e.tick(&emit);
        assert!(ev.lock().unwrap().is_empty());
        e.push(&loud(1));
        e.tick(&emit);
        assert_eq!(ev.lock().unwrap().as_slice(), &[Event::Partial("t1".into())]);
        assert_eq!(f.lens.lock().unwrap()[0], 2 * SR);
    }

    #[test]
    fn commits_long_segments_and_only_retranscribes_the_tail() {
        let (e, f, _d) = engine(false);
        let (ev, emit) = collect();
        e.push(&loud(28));
        e.tick(&emit); // commit "t1"
        e.push(&loud(2));
        e.tick(&emit); // partial over only the new 2s
        assert_eq!(f.lens.lock().unwrap().as_slice(), &[28 * SR, 2 * SR]);
        assert_eq!(ev.lock().unwrap().last(), Some(&Event::Partial("t1 t2".into())));
        let out = e.finish(&emit);
        assert_eq!(out, "t1 t3");
    }

    #[test]
    fn finish_with_no_audio_returns_empty_without_calling_whisper() {
        let (e, f, _d) = engine(false);
        let (ev, emit) = collect();
        assert_eq!(e.finish(&emit), "");
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert_eq!(ev.lock().unwrap().as_slice(), &[Event::Final(String::new())]);
    }

    #[test]
    fn transcriber_errors_surface_as_events_not_panics() {
        let (e, _f, _d) = engine(true);
        let (ev, emit) = collect();
        e.push(&loud(3));
        e.tick(&emit);
        assert!(matches!(ev.lock().unwrap()[0], Event::Error(_)));
    }

    #[test]
    fn silent_audio_never_reaches_whisper_and_explains_why() {
        let (e, f, _d) = engine(false);
        let (ev, emit) = collect();
        e.push(&vec![0i16; 3 * SR]);
        e.tick(&emit);
        e.push(&vec![0i16; 3 * SR]);
        e.tick(&emit);
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        let evs = ev.lock().unwrap();
        assert_eq!(evs.len(), 1, "warn once, not every tick: {evs:?}");
        assert!(matches!(&evs[0], Event::Error(m) if m.contains("No sound")));
        drop(evs);
        assert_eq!(e.finish(&emit), "");
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn quiet_speech_is_transcribed_and_boosted() {
        let (e, f, _d) = engine(false);
        let (_ev, emit) = collect();
        e.push(&vec![900i16; 3 * SR]); // peak ~0.027, like the real laptop mic
        e.tick(&emit);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        assert!(f.peaks.lock().unwrap()[0] > 20_000);
    }

    #[test]
    fn speech_after_silence_is_transcribed() {
        let (e, f, _d) = engine(false);
        let (_ev, emit) = collect();
        e.push(&vec![0i16; 3 * SR]);
        e.tick(&emit);
        e.push(&loud(3));
        e.tick(&emit);
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn temp_audio_is_deleted_after_use() {
        let (e, _f, d) = engine(false);
        let (_ev, emit) = collect();
        e.push(&loud(3));
        e.tick(&emit);
        e.finish(&emit);
        let left: Vec<_> = std::fs::read_dir(d.path().join("tmp")).unwrap().collect();
        assert!(left.is_empty());
    }
}

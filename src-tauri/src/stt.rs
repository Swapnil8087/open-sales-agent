//! Speech-to-text via the whisper.cpp `whisper-cli` binary (fully offline).

use crate::session::clean_transcript;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MODEL_FILE: &str = "ggml-base.en.bin";

#[derive(Debug, PartialEq, Eq)]
pub enum SttError {
    BinaryMissing(PathBuf),
    ModelMissing(PathBuf),
    Failed(String),
    Timeout,
}

impl fmt::Display for SttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SttError::BinaryMissing(p) => write!(f, "whisper-cli not found at {}", p.display()),
            SttError::ModelMissing(p) => write!(f, "Whisper model not found at {}", p.display()),
            SttError::Failed(m) => write!(f, "transcription failed: {m}"),
            SttError::Timeout => write!(f, "transcription timed out"),
        }
    }
}

pub trait Transcriber: Send + Sync {
    fn transcribe(&self, wav: &Path) -> Result<String, SttError>;
}

pub struct WhisperCli {
    pub bin: PathBuf,
    pub model: PathBuf,
    pub timeout: Duration,
}

impl WhisperCli {
    pub fn new(bin: PathBuf, model: PathBuf) -> Self {
        Self { bin, model, timeout: Duration::from_secs(60) }
    }
}

impl Transcriber for WhisperCli {
    fn transcribe(&self, wav: &Path) -> Result<String, SttError> {
        if !self.bin.is_file() {
            return Err(SttError::BinaryMissing(self.bin.clone()));
        }
        if !self.model.is_file() {
            return Err(SttError::ModelMissing(self.model.clone()));
        }
        // Fixed argv, no shell: nothing user-controlled is interpreted.
        let mut child = Command::new(&self.bin)
            .arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(wav)
            .args(["-nt", "-np", "-l", "en"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| SttError::Failed(e.to_string()))?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            s
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait().map_err(|e| SttError::Failed(e.to_string()))? {
                Some(s) => break s,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(SttError::Timeout);
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let out = reader.join().unwrap_or_default();
        if !status.success() {
            return Err(SttError::Failed(format!("exit status {status}")));
        }
        Ok(clean_transcript(&out))
    }
}

/// First candidate that exists as a file.
pub fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

pub fn whisper_bin_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("OSA_WHISPER_BIN") {
        v.push(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(dir.join("whisper-cli"));
    }
    v.push("/opt/homebrew/bin/whisper-cli".into());
    v.push("/usr/local/bin/whisper-cli".into());
    v
}

pub fn model_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Open-Sales-Agent").join("models").join(MODEL_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let model = d.path().join("m.bin");
        std::fs::write(&model, b"x").unwrap();
        let wav = d.path().join("a.wav");
        std::fs::write(&wav, b"x").unwrap();
        (d, model, wav)
    }

    #[test]
    fn returns_cleaned_stdout() {
        let (d, model, wav) = setup();
        let bin = script(d.path(), "w", "echo ' Hello [BLANK_AUDIO] world '");
        let t = WhisperCli::new(bin, model);
        assert_eq!(t.transcribe(&wav).unwrap(), "Hello world");
    }

    #[test]
    fn passes_expected_arguments() {
        let (d, model, wav) = setup();
        let bin = script(d.path(), "w", "echo \"$@\"");
        let t = WhisperCli::new(bin, model.clone());
        let out = t.transcribe(&wav).unwrap();
        assert!(out.contains(&format!("-m {}", model.display())));
        assert!(out.contains(&format!("-f {}", wav.display())));
        assert!(out.contains("-nt"));
    }

    #[test]
    fn missing_binary_and_model_are_distinct_errors() {
        let (d, model, wav) = setup();
        let t = WhisperCli::new(d.path().join("nope"), model);
        assert!(matches!(t.transcribe(&wav), Err(SttError::BinaryMissing(_))));
        let bin = script(d.path(), "w", "echo hi");
        let t = WhisperCli::new(bin, d.path().join("nomodel"));
        assert!(matches!(t.transcribe(&wav), Err(SttError::ModelMissing(_))));
    }

    #[test]
    fn nonzero_exit_is_failure() {
        let (d, model, wav) = setup();
        let bin = script(d.path(), "w", "exit 3");
        assert!(matches!(WhisperCli::new(bin, model).transcribe(&wav), Err(SttError::Failed(_))));
    }

    #[test]
    fn hung_process_is_killed_on_timeout() {
        let (d, model, wav) = setup();
        let bin = script(d.path(), "w", "sleep 30");
        let mut t = WhisperCli::new(bin, model);
        t.timeout = Duration::from_millis(200);
        let start = Instant::now();
        assert_eq!(t.transcribe(&wav), Err(SttError::Timeout));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn first_existing_picks_first_real_file() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::write(&a, "x").unwrap();
        let got = first_existing(&[d.path().join("zz"), a.clone()]);
        assert_eq!(got, Some(a));
        assert_eq!(first_existing(&[d.path().join("zz")]), None);
    }
}

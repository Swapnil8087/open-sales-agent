//! Live-transcription session: decides when to transcribe and stitches text.

pub const SR: usize = crate::audio::SAMPLE_RATE as usize;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Step {
    Wait,
    Partial,
    Commit,
}

/// Pure scheduling decision. `total`/`seg_start`/`last_attempt` are sample counts.
pub fn plan(total: usize, seg_start: usize, last_attempt: usize, tail_silent: bool) -> Step {
    let seg_len = total.saturating_sub(seg_start);
    if seg_len >= 28 * SR || (seg_len >= 15 * SR && tail_silent) {
        return Step::Commit;
    }
    if seg_len >= SR / 2 && total.saturating_sub(last_attempt) >= 2 * SR {
        return Step::Partial;
    }
    Step::Wait
}

/// Strip whisper artefacts like `[BLANK_AUDIO]`, `(silence)`, `[Music]`.
pub fn clean_transcript(raw: &str) -> String {
    const NOISE: [&str; 6] = ["silence", "music", "blank_audio", "noise", "applause", "inaudible"];
    let mut out = String::new();
    let mut rest = raw;
    while let Some(i) = rest.find(['[', '(']) {
        out.push_str(&rest[..i]);
        let close = if rest[i..].starts_with('[') { ']' } else { ')' };
        match rest[i..].find(close) {
            Some(j) => {
                let inner = rest[i + 1..i + j].trim().to_lowercase();
                let drop = close == ']' || NOISE.contains(&inner.as_str());
                if !drop {
                    out.push_str(&rest[i..=i + j]);
                }
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // Classic whisper hallucination on near-silence: nothing but "you".
    let only_you = !out.is_empty()
        && out.split_whitespace().all(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).eq_ignore_ascii_case("you"));
    if only_you { String::new() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_until_two_seconds_of_new_audio() {
        assert_eq!(plan(SR, 0, 0, false), Step::Wait);
        assert_eq!(plan(2 * SR, 0, 0, false), Step::Partial);
    }

    #[test]
    fn does_not_retranscribe_too_soon() {
        assert_eq!(plan(3 * SR, 0, 2 * SR, false), Step::Wait);
        assert_eq!(plan(4 * SR, 0, 2 * SR, false), Step::Partial);
    }

    #[test]
    fn commits_on_pause_after_fifteen_seconds() {
        assert_eq!(plan(16 * SR, 0, 16 * SR, true), Step::Commit);
        assert_eq!(plan(16 * SR, 0, 16 * SR, false), Step::Wait);
    }

    #[test]
    fn force_commits_at_28_seconds() {
        assert_eq!(plan(28 * SR, 0, 28 * SR, false), Step::Commit);
    }

    #[test]
    fn segment_length_is_relative_to_seg_start() {
        assert_eq!(plan(40 * SR, 30 * SR, 40 * SR, false), Step::Wait);
        assert_eq!(plan(60 * SR, 30 * SR, 58 * SR, false), Step::Commit);
    }

    #[test]
    fn cleans_whisper_artifacts() {
        assert_eq!(clean_transcript("[BLANK_AUDIO]"), "");
        assert_eq!(clean_transcript(" Hello [Music] world (silence) "), "Hello world");
        assert_eq!(clean_transcript("line one\n  line two\n"), "line one line two");
        assert_eq!(clean_transcript("you you you"), "");
        assert_eq!(clean_transcript(" You. You. "), "");
        assert_eq!(clean_transcript("Thank you."), "Thank you.");
        assert_eq!(clean_transcript("Are you in?"), "Are you in?");
        assert_eq!(clean_transcript("Budget is (about) 5 crore"), "Budget is (about) 5 crore");
    }
}

import { useCallback, useEffect, useRef, useState } from "react";
import {
  hideWindow,
  onAudioLevel,
  onBarShown,
  onPartialTranscript,
  onTranscriptError,
  openDataFolder,
  resizeBar,
  startRecording,
  stopRecording,
} from "../lib/tauri";
import Waveform from "./Waveform";

type Mode = "ask" | "email" | "mom" | "inquiry";

const MODES: Record<Mode, { placeholder: string; multiline: boolean }> = {
  ask: { placeholder: "Ask anything…", multiline: false },
  email: { placeholder: "Describe the email to draft (client, topic, tone)…", multiline: false },
  mom: { placeholder: "Paste or dictate meeting notes for the MoM…", multiline: true },
  inquiry: { placeholder: "Add inquiry details: client, space needed, budget, timeline…", multiline: true },
};

const ACTIONS: { label: string; mode: Mode }[] = [
  { label: "Inquiry", mode: "inquiry" },
  { label: "Email", mode: "email" },
  { label: "MoM", mode: "mom" },
];

const btn =
  "rounded-lg px-3 py-1.5 text-sm text-neutral-200 bg-white/5 hover:bg-white/10 transition-colors";
const btnActive = "bg-indigo-500/30 text-white";

const join = (a: string, b: string) => (a && b ? `${a} ${b}` : a || b);

export default function CommandBar() {
  const [mode, setMode] = useState<Mode>("ask");
  const [text, setText] = useState("");
  const [recording, setRecording] = useState(false);
  const [busy, setBusy] = useState(false); // final transcription pass running
  const [level, setLevel] = useState(0);
  const recordingRef = useRef(false);
  const baseRef = useRef(""); // text typed before recording started
  const textRef = useRef("");
  textRef.current = text;
  const [response, setResponse] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const fieldRef = useRef<HTMLInputElement & HTMLTextAreaElement>(null);

  const reset = useCallback(() => {
    // The bar may be hidden mid-meeting; never wipe an active recording.
    if (recordingRef.current) return;
    setMode("ask");
    setText("");
    setResponse(null);
    setTimeout(() => fieldRef.current?.focus(), 0);
  }, []);

  // Fresh state every time the bar is summoned.
  useEffect(() => {
    let un: (() => void) | undefined;
    onBarShown(reset).then((u) => (un = u));
    return () => un?.();
  }, [reset]);

  // Live transcription from the Rust engine.
  useEffect(() => {
    const unlisten = [
      onPartialTranscript((t) => {
        if (recordingRef.current) setText(join(baseRef.current, t));
      }),
      onAudioLevel((v) => {
        if (recordingRef.current) setLevel(v);
      }),
      onTranscriptError((m) => setResponse(m)),
    ];
    return () => unlisten.forEach((p) => void p.then((u) => u()));
  }, []);

  const toggleRecording = async () => {
    if (busy) return;
    if (recordingRef.current) {
      setBusy(true);
      try {
        const finalText = await stopRecording();
        setText(join(baseRef.current, finalText));
      } catch (e) {
        setResponse(String(e));
      } finally {
        recordingRef.current = false;
        setRecording(false);
        setLevel(0);
        setBusy(false);
      }
      return;
    }
    baseRef.current = textRef.current;
    try {
      await startRecording();
      recordingRef.current = true;
      setRecording(true);
      setResponse(null);
    } catch (e) {
      setResponse(String(e));
    }
  };
  const toggleRef = useRef(toggleRecording);
  toggleRef.current = toggleRecording;

  // Window hugs content so transparent areas never block clicks.
  useEffect(() => {
    const el = rootRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => void resizeBar(el.getBoundingClientRect().height));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // `M` toggles recording when not typing; Alt+M works anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== "m" || e.metaKey || e.ctrlKey) return;
      const el = document.activeElement;
      const typing = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;
      if (e.altKey || !typing) {
        e.preventDefault();
        void toggleRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const pickMode = (m: Mode) => {
    setMode(m);
    setResponse(null);
    setTimeout(() => fieldRef.current?.focus(), 0);
  };

  const submit = () => {
    if (!text.trim()) return;
    // Phase 4 replaces this stub with the local Ollama call.
    setResponse("Local model not connected yet (coming in Phase 4).");
  };

  const onFieldKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      if (response !== null) setResponse(null);
      else void hideWindow();
    } else if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  const fieldProps = {
    ref: fieldRef,
    autoFocus: true,
    value: text,
    placeholder: MODES[mode].placeholder,
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setText(e.target.value),
    onKeyDown: onFieldKey,
    className:
      "w-full resize-none bg-transparent text-base text-white placeholder-neutral-500 outline-none",
  };

  return (
    <div
      ref={rootRef}
      className="m-1 overflow-hidden rounded-2xl border border-white/10 bg-neutral-900/95 text-white shadow-2xl backdrop-blur-xl"
    >
      <div className="flex items-center gap-3 px-4 py-3">
        {recording ? <Waveform level={level} /> : null}
        <div className="flex-1">
          {MODES[mode].multiline ? (
            <textarea {...fieldProps} rows={3} />
          ) : (
            <input {...fieldProps} type="text" />
          )}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 border-t border-white/5 px-3 py-2">
        <button
          className={`${btn} ${recording ? "bg-rose-500/30 text-white" : ""}`}
          disabled={busy}
          onClick={() => void toggleRecording()}
        >
          {busy ? "Transcribing…" : recording ? "Stop" : "Mic"}
        </button>
        {ACTIONS.map((a) => (
          <button
            key={a.label}
            className={`${btn} ${mode === a.mode ? btnActive : ""}`}
            onClick={() => pickMode(a.mode)}
          >
            {a.label}
          </button>
        ))}
        <button className={`${btn} ml-auto`} onClick={() => void openDataFolder()}>
          Open Folder
        </button>
      </div>

      {response !== null && (
        <div data-testid="response" className="border-t border-white/5 px-4 py-3 text-sm text-neutral-300">
          {response}
        </div>
      )}
    </div>
  );
}

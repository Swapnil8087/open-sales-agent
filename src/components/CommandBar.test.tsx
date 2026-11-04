import { render, screen, fireEvent, act } from "@testing-library/react";
import { describe, test, expect, vi, beforeEach } from "vitest";

const { shown, ev, sub } = vi.hoisted(() => {
  const shown: { cb: () => void } = { cb: () => {} };
  const ev: Record<string, (v: never) => void> = {};
  const sub = (name: string) =>
    vi.fn(async (cb: (v: never) => void) => {
      ev[name] = cb;
      return () => {};
    });
  return { shown, ev, sub };
});
vi.mock("../lib/tauri", () => ({
  hideWindow: vi.fn(),
  openDataFolder: vi.fn(),
  resizeBar: vi.fn(),
  startRecording: vi.fn(async () => {}),
  stopRecording: vi.fn(async () => "final words"),
  onAudioLevel: sub("level"),
  onPartialTranscript: sub("partial"),
  onTranscriptError: sub("error"),
  onBarShown: vi.fn(async (cb: () => void) => {
    shown.cb = cb;
    return () => {};
  }),
}));

import * as tauri from "../lib/tauri";
import CommandBar from "./CommandBar";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(tauri.startRecording).mockResolvedValue(undefined);
  vi.mocked(tauri.stopRecording).mockResolvedValue("final words");
});

const emit = (name: string, v: unknown) => act(() => ev[name](v as never));
const click = async (name: string) => {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
  });
};

const input = () => screen.getByRole("textbox");

describe("CommandBar", () => {
  test("shows the five quick-action buttons", () => {
    render(<CommandBar />);
    for (const name of ["Mic", "Inquiry", "Email", "MoM", "Open Folder"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
  });

  test("clicking Email switches mode and placeholder, focuses input", () => {
    render(<CommandBar />);
    fireEvent.click(screen.getByRole("button", { name: "Email" }));
    expect(input()).toHaveAttribute("placeholder", expect.stringMatching(/email/i));
    expect(input()).toHaveFocus();
  });

  test("Inquiry mode offers a multi-line details box", () => {
    render(<CommandBar />);
    fireEvent.click(screen.getByRole("button", { name: "Inquiry" }));
    expect(input().tagName).toBe("TEXTAREA");
  });

  test("mic button starts the backend recorder and shows the visualizer", async () => {
    render(<CommandBar />);
    expect(screen.queryByRole("img", { name: /audio/i })).toBeNull();
    await click("Mic");
    expect(tauri.startRecording).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("img", { name: /audio/i })).toBeInTheDocument();
    await click("Stop");
    expect(tauri.stopRecording).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("img", { name: /audio/i })).toBeNull();
  });

  test("live partial transcript streams into the text box", async () => {
    render(<CommandBar />);
    await act(async () => {});
    await click("Mic");
    emit("partial", "we need");
    expect(input()).toHaveValue("we need");
    emit("partial", "we need 50 seats");
    expect(input()).toHaveValue("we need 50 seats");
  });

  test("final transcript replaces partials and keeps text typed before recording", async () => {
    render(<CommandBar />);
    await act(async () => {});
    fireEvent.change(input(), { target: { value: "Client Raj:" } });
    await click("Mic");
    emit("partial", "need parking");
    expect(input()).toHaveValue("Client Raj: need parking");
    await click("Stop");
    expect(input()).toHaveValue("Client Raj: final words");
  });

  test("audio level events are ignored when not recording", async () => {
    render(<CommandBar />);
    await act(async () => {});
    emit("partial", "stray");
    expect(input()).toHaveValue("");
  });

  test("shows a helpful message when the recorder cannot start", async () => {
    vi.mocked(tauri.startRecording).mockRejectedValue("Whisper engine not installed");
    render(<CommandBar />);
    await click("Mic");
    expect(screen.getByTestId("response")).toHaveTextContent("Whisper engine not installed");
    expect(screen.queryByRole("img", { name: /audio/i })).toBeNull();
    expect(screen.getByRole("button", { name: "Mic" })).toBeInTheDocument();
  });

  test("non-fatal transcription errors are displayed", async () => {
    render(<CommandBar />);
    await act(async () => {});
    await click("Mic");
    emit("error", "transcription failed: exit status 1");
    expect(screen.getByTestId("response")).toHaveTextContent("transcription failed");
  });

  test("re-showing the bar keeps an active recording", async () => {
    render(<CommandBar />);
    await act(async () => {});
    await click("Mic");
    emit("partial", "still talking");
    act(() => shown.cb());
    expect(input()).toHaveValue("still talking");
    expect(screen.getByRole("img", { name: /audio/i })).toBeInTheDocument();
  });

  test("M toggles recording only when focus is outside the text field", async () => {
    render(<CommandBar />);
    fireEvent.keyDown(input(), { key: "m" });
    expect(tauri.startRecording).not.toHaveBeenCalled();
    (document.activeElement as HTMLElement).blur();
    await act(async () => {
      fireEvent.keyDown(document.body, { key: "m" });
    });
    expect(tauri.startRecording).toHaveBeenCalledTimes(1);
  });

  test("Alt+M toggles recording even while typing", async () => {
    render(<CommandBar />);
    await act(async () => {
      fireEvent.keyDown(input(), { key: "m", altKey: true });
    });
    expect(tauri.startRecording).toHaveBeenCalledTimes(1);
  });

  test("submitting opens the response area; Esc collapses it, then hides", () => {
    render(<CommandBar />);
    fireEvent.change(input(), { target: { value: "hello" } });
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(screen.getByTestId("response")).toBeInTheDocument();
    fireEvent.keyDown(input(), { key: "Escape" });
    expect(screen.queryByTestId("response")).toBeNull();
    expect(tauri.hideWindow).not.toHaveBeenCalled();
    fireEvent.keyDown(input(), { key: "Escape" });
    expect(tauri.hideWindow).toHaveBeenCalledTimes(1);
  });

  test("empty submit does nothing", () => {
    render(<CommandBar />);
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(screen.queryByTestId("response")).toBeNull();
  });

  test("Open Folder calls backend", () => {
    render(<CommandBar />);
    fireEvent.click(screen.getByRole("button", { name: "Open Folder" }));
    expect(tauri.openDataFolder).toHaveBeenCalled();
  });

  test("re-showing the bar resets state", async () => {
    render(<CommandBar />);
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: "Email" }));
    fireEvent.change(input(), { target: { value: "draft" } });
    act(() => shown.cb());
    expect(input()).toHaveValue("");
    expect(input()).toHaveAttribute("placeholder", expect.stringMatching(/ask/i));
  });
});

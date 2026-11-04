import { render, screen, fireEvent, act } from "@testing-library/react";
import { describe, test, expect, vi, beforeEach } from "vitest";

const shown: { cb: () => void } = { cb: () => {} };
vi.mock("../lib/tauri", () => ({
  hideWindow: vi.fn(),
  openDataFolder: vi.fn(),
  resizeBar: vi.fn(),
  onBarShown: vi.fn(async (cb: () => void) => {
    shown.cb = cb;
    return () => {};
  }),
}));

import * as tauri from "../lib/tauri";
import CommandBar from "./CommandBar";

beforeEach(() => vi.clearAllMocks());

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

  test("mic button toggles recording and the visualizer", () => {
    render(<CommandBar />);
    expect(screen.queryByRole("img", { name: /audio/i })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Mic" }));
    expect(screen.getByRole("img", { name: /audio/i })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(screen.queryByRole("img", { name: /audio/i })).toBeNull();
  });

  test("M toggles recording only when focus is outside the text field", () => {
    render(<CommandBar />);
    fireEvent.keyDown(input(), { key: "m" });
    expect(screen.queryByRole("img", { name: /audio/i })).toBeNull();
    (document.activeElement as HTMLElement).blur();
    fireEvent.keyDown(document.body, { key: "m" });
    expect(screen.getByRole("img", { name: /audio/i })).toBeInTheDocument();
  });

  test("Alt+M toggles recording even while typing", () => {
    render(<CommandBar />);
    fireEvent.keyDown(input(), { key: "m", altKey: true });
    expect(screen.getByRole("img", { name: /audio/i })).toBeInTheDocument();
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

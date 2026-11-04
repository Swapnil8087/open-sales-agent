import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

export const BAR_WIDTH = 680;

export const hideWindow = () => invoke<void>("hide_window");
export const openDataFolder = () => invoke<void>("open_data_folder");

/** Resize the (transparent) window to hug the content. */
export const resizeBar = (height: number) =>
  getCurrentWindow().setSize(new LogicalSize(BAR_WIDTH, Math.ceil(height)));

/** Fires each time the hotkey/tray shows the bar. Returns an unlisten fn. */
export const onBarShown = async (cb: () => void) => listen("bar:shown", cb);

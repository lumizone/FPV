/**
 * Platform detection utilities for conditional UI rendering.
 *
 * Uses the User-Agent string from the WebView — Tauri's WebView2 on
 * Windows reports "Windows", macOS WebKit reports "Macintosh".
 * This is cheaper than a Tauri command round-trip and available
 * synchronously (no async needed).
 */

const ua = navigator.userAgent;

export const isMac = ua.includes("Macintosh") || ua.includes("Mac OS");
export const isWindows = ua.includes("Windows");
export const isLinux = ua.includes("Linux") && !ua.includes("Android");

/** Modifier key label — ⌘ on Mac, Ctrl+ on Windows/Linux. Both `modKey`
 *  and `modShift` are meant to be prepended directly to a key label
 *  (`` `${modKey}F` `` / `` `${modShift}V` ``), so both carry the same
 *  trailing-separator convention: none needed on Mac (symbols prefix
 *  straight onto the letter), a trailing `+` on Windows/Linux. */
export const modKey = isMac ? "⌘" : "Ctrl+";

/** Modifier key label for display in shortcut hints */
export const modShift = isMac ? "⌘⇧" : "Ctrl+Shift+";

/**
 * Whether Apple-native integrations (Calendar via EventKit, Reminders,
 * Vision OCR) are available. On Windows/Linux these return NotImplemented
 * from the backend; UI should hide or grey them out.
 */
export const hasAppleIntegrations = isMac;

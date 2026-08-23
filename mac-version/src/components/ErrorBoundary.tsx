// Top-level React error boundary. A single misbehaving component
// (an updater plugin throwing because the endpoint is down, a Tauri
// channel returning unexpected JSON) used to bring down the whole UI
// tree. This catches crashes and offers a reload path that doesn't
// touch Rust state, so stories/worlds/license all survive.
//
// Errors are also forwarded to console + the Rust side via a custom
// log_renderer_crash command so the crash.log helper picks them up.

import { Component, type ErrorInfo, type ReactNode } from "react";
import { logRendererCrash } from "@/lib/tauri";
import i18n from "@/i18n/config";

interface Props {
  children: ReactNode;
}

interface State {
  err: Error | null;
  info: ErrorInfo | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { err: null, info: null };

  static getDerivedStateFromError(err: Error): State {
    return { err, info: null };
  }

  componentDidCatch(err: Error, info: ErrorInfo) {
    this.setState({ info });
    // Best-effort: ship the stack to the Rust crash log. Failure
    // here cannot prevent the boundary from rendering — wrap in
    // try/catch and swallow.
    try {
      logRendererCrash(
        err.message,
        err.stack ?? "(no stack)",
        info.componentStack ?? "(no component stack)"
      ).catch(() => {});
    } catch (_) {
      // tauri not available (browser preview etc.) — ignore
    }
    // eslint-disable-next-line no-console
    console.error("[ErrorBoundary] caught", err, info);
  }

  reset = () => {
    this.setState({ err: null, info: null });
  };

  hardReload = () => {
    window.location.reload();
  };

  render() {
    if (!this.state.err) return this.props.children;
    return (
      <div className="fixed inset-0 z-[200] bg-[var(--color-bg-content)] text-[var(--color-label-primary)] flex items-center justify-center p-8">
        <div className="max-w-md w-full space-y-4">
          <div className="text-[24px] font-serif text-[var(--color-warm)] mb-1">✦</div>
          <h1 className="text-[20px] font-display tracking-[0.02em]">{i18n.t("errors.title", "Something went wrong")}</h1>
          <p className="text-[13px] text-[var(--color-label-secondary)]">
            {i18n.t("errors.body", "A piece of the interface failed. Your stories and worlds are safe. Try again or reload the window.")}
          </p>
          <p className="text-[11px] text-[var(--color-label-tertiary)]">
            {i18n.t("errors.diagnostic", "A diagnostic entry was saved locally.")} {i18n.t("errors.body", "Try again or reload the window.")}
          </p>
          <div className="flex gap-2">
            <button
              onClick={this.reset}
              className="flex-1 px-4 py-2 rounded-xl bg-[var(--color-accent)] text-black font-semibold text-[13px] transition-colors"
            >
              {i18n.t("errors.retry", "Try again")}
            </button>
            <button
              onClick={this.hardReload}
              className="flex-1 px-4 py-2 rounded-xl border border-[var(--color-separator)] text-[var(--color-label-primary)] hover:bg-[var(--color-fill-quaternary)] text-[13px] font-medium transition-colors"
            >
              {i18n.t("errors.reload", "Reload window")}
            </button>
          </div>
        </div>
      </div>
    );
  }
}

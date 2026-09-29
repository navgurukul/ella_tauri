import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import App from "./App";
import { bridge } from "./lib/bridge";
import { SetupScreen, amountOf, timeLeft, troubleFor, viewFor } from "./components/SetupScreen";
import type { SetupState } from "./lib/setup";
import type { ApplyUpdate, UpdateProgress } from "./lib/updates";

// The updater would otherwise go through the mocked IPC too; these tests
// drive its progress by hand, as UpdateToast.test.tsx does.
const updater = vi.hoisted(() => ({
  report: null as ((progress: UpdateProgress | null) => void) | null,
  finish: null as ((apply: ApplyUpdate | null) => void) | null,
}));
vi.mock("./lib/updates", () => ({
  installExitsTheApp: () => true,
  downloadUpdateInBackground: (report: (progress: UpdateProgress | null) => void) => {
    updater.report = report;
    return new Promise<ApplyUpdate | null>((resolve) => {
      updater.finish = resolve;
    });
  },
}));

const GB = 1024 ** 3;

function progress(stage: SetupState["stage"], seq: number, extra: Partial<SetupState> = {}): SetupState {
  return {
    stage,
    message: "",
    downloaded_bytes: 0,
    total_bytes: 0,
    index: 0,
    of: 0,
    attempt: 1,
    first_run: false,
    fell_back: false,
    failure: null,
    trouble: null,
    seq,
    ...extra,
  };
}

const downloading = (seq: number, extra: Partial<SetupState> = {}) =>
  progress("downloading", seq, {
    downloaded_bytes: 1.2 * GB,
    total_bytes: 2.3 * GB,
    index: 1,
    of: 2,
    first_run: true,
    ...extra,
  });

/** What the mocked backend answers. The bootstrap stays on the browser
 * bridge: bridge.ts chose it at import, before these mocks existed. */
const backend = {
  setupState: (): SetupState | Promise<SetupState> => progress("ready", 1),
  retrySetup: vi.fn((): SetupState => progress("ready", 1)),
};

/** The setup event, as the Rust side sends it. */
async function announce(state: SetupState) {
  await act(async () => {
    await emit("ella://setup", state);
  });
}

const letsStart = () => screen.queryByRole("button", { name: /let’s start/i });

beforeEach(() => {
  window.localStorage.clear();
  backend.setupState = () => progress("ready", 1);
  backend.retrySetup = vi.fn(() => progress("ready", 1));
  mockIPC(
    (command) => {
      if (command === "setup_state") return backend.setupState();
      if (command === "retry_setup") return backend.retrySetup();
      throw new Error(`unexpected command ${command}`);
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  vi.useRealTimers();
  // Unmount while the mocks still stand, so the setup hook can unlisten.
  cleanup();
  clearMocks();
  // clearMocks empties the object but leaves it there, and the setup hook
  // would then think every later test runs inside Tauri.
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  delete (window as { __TAURI_EVENT_PLUGIN_INTERNALS__?: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__;
});

describe("the setup screen", () => {
  it("is all there is while Ella downloads: no way into the app", async () => {
    backend.setupState = () => downloading(5);
    render(<App />);

    expect(await screen.findByRole("heading", { name: "Getting Ella ready" })).toBeInTheDocument();
    expect(screen.getByText("1.2 of 2.3 GB")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Download" })).toHaveAttribute("aria-valuenow", "52");
    expect(letsStart()).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /log in/i })).not.toBeInTheDocument();
    expect(document.querySelector('[data-screen^="onboarding"]')).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Talk partners" })).not.toBeInTheDocument();
  });

  it("walks a new learner through the download and the first load to the welcome", async () => {
    backend.setupState = () => downloading(5);
    render(<App />);
    await screen.findByRole("heading", { name: "Getting Ella ready" });

    // The backend keeps `first_run` on every announcement after a download.
    await announce(progress("loading", 6, { first_run: true }));
    expect(screen.getByRole("heading", { name: "Almost ready!" })).toBeInTheDocument();
    expect(letsStart()).not.toBeInTheDocument();

    await announce(progress("ready", 7));
    // A moment to say so, then the welcome.
    expect(screen.getByRole("heading", { name: "Ella is ready!" })).toBeInTheDocument();
    expect(letsStart()).not.toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /let’s start/i }, { timeout: 3000 })).toBeInTheDocument();
  });

  it("keeps a signed-in learner out too, then goes straight home on an ordinary start", async () => {
    await bridge.saveLearner("Asha", 12);
    backend.setupState = () => progress("loading", 2);
    render(<App />);

    expect(await screen.findByText("Hi Asha! She’ll be ready in a moment.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Waking Ella up…" })).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.queryByText("Namaste, Asha!")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Talk partners" })).not.toBeInTheDocument();

    await announce(progress("ready", 3));
    // No "Ella is ready!" moment on an ordinary start.
    expect(await screen.findByText("Namaste, Asha!")).toBeInTheDocument();
  });

  it("opens at once when setup was already done before the window asked", async () => {
    render(<App />);
    expect(await screen.findByRole("button", { name: /let’s start/i })).toBeInTheDocument();
  });

  it("does not lose an announcement made between listening and asking", async () => {
    backend.setupState = () => {
      // The ready event lands first; the answer, from just before it, second.
      void emit("ella://setup", progress("ready", 3));
      return progress("downloading", 2);
    };
    render(<App />);
    expect(await screen.findByRole("button", { name: /let’s start/i }, { timeout: 3000 })).toBeInTheDocument();
  });

  it("ignores an older answer that arrives after a newer event", async () => {
    let answer: (state: SetupState) => void = () => undefined;
    backend.setupState = () => new Promise<SetupState>((resolve) => (answer = resolve));
    render(<App />);
    await screen.findByRole("heading", { name: "Waking Ella up…" });

    await announce(progress("ready", 9));
    await act(async () => answer(downloading(8)));
    expect(await screen.findByRole("button", { name: /let’s start/i })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Getting Ella ready" })).not.toBeInTheDocument();
  });

  it("never closes again once the learner is in", async () => {
    render(<App />);
    await screen.findByRole("button", { name: /let’s start/i });
    await announce(progress("failed", 9, { failure: "load", message: "gone" }));
    expect(letsStart()).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Ella couldn’t wake up" })).not.toBeInTheDocument();
  });

  it("keeps asking while it waits, so a lost ready event cannot strand the learner", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    let calls = 0;
    backend.setupState = () => (++calls === 1 ? downloading(5) : progress("ready", 7, { first_run: true }));
    render(<App />);
    // The first answer is in, and no event will ever follow it.
    await screen.findByRole("heading", { name: "Getting Ella ready" });

    await act(async () => {
      vi.advanceTimersByTime(3100);
    });
    expect(await screen.findByRole("heading", { name: "Ella is ready!" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /let’s start/i }, { timeout: 3000 })).toBeInTheDocument();
  });
});

describe("when setup fails", () => {
  const stopped = (seq: number, message = "error sending request for url") =>
    progress("failed", seq, {
      failure: "download",
      message,
      downloaded_bytes: 1.2 * GB,
      total_bytes: 2.3 * GB,
      first_run: true,
    });

  it("shows what was saved and one way forward: Try again", async () => {
    backend.setupState = () => stopped(4);
    backend.retrySetup = vi.fn(() => downloading(5, { message: "Trying the download again" }));
    render(<App />);

    expect(await screen.findByRole("heading", { name: "The download paused" })).toBeInTheDocument();
    expect(screen.getByText("1.2 of 2.3 GB saved")).toBeInTheDocument();
    const retry = screen.getByRole("button", { name: /try again/i });
    expect(retry).toHaveFocus();
    expect(letsStart()).not.toBeInTheDocument();

    fireEvent.click(retry);
    fireEvent.click(retry);
    expect(backend.retrySetup).toHaveBeenCalledOnce();
    expect(await screen.findByRole("heading", { name: "Getting Ella ready" })).toBeInTheDocument();
    expect(screen.getByText("1.2 of 2.3 GB")).toBeInTheDocument();
  });

  it("brings the button back when a press is refused", async () => {
    backend.setupState = () => stopped(4);
    backend.retrySetup = vi.fn(() => stopped(4));
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: /try again/i }));
    const again = await screen.findByRole("button", { name: /try again/i });
    expect(again).not.toHaveAttribute("aria-disabled");
  });

  it("does not claim anything was saved when nothing downloaded", async () => {
    backend.setupState = () => ({ ...stopped(4, "error sending request: dns error"), downloaded_bytes: 0 });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Ella can’t reach the internet" })).toBeInTheDocument();
    expect(screen.queryByText(/saved/)).not.toBeInTheDocument();
  });

  it("tells a teacher when the school network is blocking the download", async () => {
    backend.setupState = () => ({ ...stopped(4, "Downloading llm failed with HTTP 403 Forbidden"), trouble: "blocked" });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "This network blocks Ella" })).toBeInTheDocument();
    expect(screen.getByText(/phone hotspot/)).toBeInTheDocument();
  });

  it("does not claim a fresh download when an update fell back to the models Ella had", async () => {
    await bridge.saveLearner("Asha", 12);
    backend.setupState = () => downloading(5, { downloaded_bytes: 0 });
    render(<App />);
    await screen.findByRole("heading", { name: "Getting Ella ready" });

    await announce(progress("loading", 6, { first_run: true, fell_back: true }));
    expect(screen.getByRole("heading", { name: "Waking Ella up…" })).toBeInTheDocument();
    expect(screen.getByText(/update will wait for next time/)).toBeInTheDocument();
    expect(screen.queryByText(/Everything is downloaded/)).not.toBeInTheDocument();

    await announce(progress("ready", 7, { first_run: true, fell_back: true }));
    expect(screen.getByRole("heading", { name: "Ella is ready!" })).toBeInTheDocument();
  });

  it("says when the laptop is full, and how much room Ella needs", async () => {
    backend.setupState = () => stopped(4, "No space left on device (os error 28)");
    render(<App />);
    expect(await screen.findByRole("heading", { name: "This laptop is full" })).toBeInTheDocument();
    expect(screen.getByText(/Free up about 1\.6 GB/)).toBeInTheDocument();
  });

  it("explains a model that would not load, with the backend's reason tucked away", async () => {
    backend.setupState = () =>
      progress("failed", 4, { failure: "load", message: "Language model: llama-server stopped before it was ready" });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Ella couldn’t wake up" })).toBeInTheDocument();
    expect(screen.getByText("Details")).toBeInTheDocument();
    expect(screen.getByText(/llama-server stopped/)).not.toBeVisible();
    expect(letsStart()).not.toBeInTheDocument();
  });

  it("stays shut if the backend cannot be asked at all", async () => {
    backend.setupState = () => {
      throw new Error("setup_state is not available");
    };
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Ella couldn’t wake up" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /try again/i })).toBeInTheDocument();
    expect(letsStart()).not.toBeInTheDocument();
  });
});

describe("the update toast on the setup screen", () => {
  it("still shows, but offers no Restart until the learner is in", async () => {
    backend.setupState = () => downloading(5);
    render(<App />);
    await screen.findByRole("heading", { name: "Getting Ella ready" });

    await act(async () => {
      updater.report?.({ stage: "ready", version: "0.1.8", downloadedBytes: 100, totalBytes: 100 });
      updater.finish?.(async () => undefined);
    });
    expect(await screen.findByText("Update ready")).toBeInTheDocument();
    expect(screen.getByText("Installs when you close Ella")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restart" })).not.toBeInTheDocument();
  });
});

describe("the time left", () => {
  it("does not count a reconnect's pause as download time", () => {
    vi.useFakeTimers();
    const MB = 1024 ** 2;
    const reconnecting = downloading(5, {
      attempt: 2,
      trouble: "network",
      downloaded_bytes: 1 * GB,
      total_bytes: 2 * GB,
    });
    const screenAt = (state: SetupState) => (
      <SetupScreen
        setup={state}
        booted
        bootError={null}
        learnerName={null}
        retrying={false}
        onRetry={() => undefined}
        onOpen={() => undefined}
      />
    );
    const { rerender } = render(screenAt(reconnecting));
    expect(screen.getByText(/Connection dropped/)).toBeInTheDocument();

    // The backoff pause, then bytes at 1 MB a second, reported every second.
    act(() => vi.advanceTimersByTime(20_000));
    for (let second = 1; second <= 10; second++) {
      act(() => vi.advanceTimersByTime(1000));
      rerender(screenAt({ ...reconnecting, downloaded_bytes: 1 * GB + second * MB, seq: 5 + second }));
    }
    act(() => vi.advanceTimersByTime(1000));

    // 1014 MB left at 1 MB/s, not the pause averaged in.
    expect(screen.getByText("About 17 minutes left")).toBeInTheDocument();
    expect(screen.queryByText(/Connection dropped/)).not.toBeInTheDocument();
  });
});

describe("setup screen wording", () => {
  it("says the unit once when both sides of the amount share it", () => {
    expect(amountOf({ downloaded_bytes: 1.34 * GB, total_bytes: 2.31 * GB })).toBe("1.3 of 2.3 GB");
    expect(amountOf({ downloaded_bytes: 640 * 1024 ** 2, total_bytes: 2.31 * GB })).toBe("640 MB of 2.3 GB");
  });

  it("reads the time left in whole, honest steps", () => {
    expect(timeLeft(null)).toBe("Working out the time left…");
    expect(timeLeft(20)).toBe("Less than a minute left");
    expect(timeLeft(61)).toBe("About 2 minutes left");
    expect(timeLeft(60 * 60)).toBe("About 60 minutes left");
    expect(timeLeft(3 * 60 * 60)).toBe("About 3 hours left");
  });

  it("takes the backend's word for what stopped a download", () => {
    expect(troubleFor({ trouble: "blocked", message: "error sending request" })).toBe("blocked");
  });

  it("reads the message only when the backend did not say", () => {
    const from = (message: string) => troubleFor({ trouble: null, message });
    expect(from("No space left on device (os error 28)")).toBe("disk");
    expect(from("There is not enough space on the disk. (os error 112)")).toBe("disk");
    expect(from("Downloading llm failed with HTTP 429 Too Many Requests")).toBe("busy");
    expect(from("Downloading llm failed with HTTP 503 Service Unavailable")).toBe("busy");
    expect(from("Downloading llm failed with HTTP 403 Forbidden")).toBe("blocked");
    expect(from("stt failed its checksum. Expected a, got b.")).toBe("broken");
    expect(from("error sending request for url")).toBe("network");
  });

  it("treats an ordinary start's load as waking up, and the end of an install as setup", () => {
    expect(viewFor(null, null, false)).toBe("waking");
    expect(viewFor(progress("loading", 2), null, false)).toBe("waking");
    expect(viewFor(progress("loading", 2, { first_run: true }), null, false)).toBe("loading");
    expect(viewFor(progress("ready", 3), null, false)).toBe("waking");
    expect(viewFor(progress("ready", 3), null, true)).toBe("ready");
    expect(viewFor(progress("ready", 3), "bootstrap failed", false)).toBe("boot-error");
  });
});

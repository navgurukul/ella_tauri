import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The updater's steps go to latency.jsonl through `record_update`; these tests
// stand in for the Tauri updater and read what was recorded.
const tauri = vi.hoisted(() => ({
  check: vi.fn(),
  invoke: vi.fn(async (_command: string, _args?: Record<string, unknown>): Promise<void> => undefined),
  relaunch: vi.fn(async () => undefined),
  closeHandler: null as (() => Promise<void>) | null,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: tauri.relaunch }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: tauri.check }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: async (handler: () => Promise<void>) => {
      tauri.closeHandler = handler;
      return () => undefined;
    },
  }),
}));

import { downloadUpdateInBackground } from "./updates";

type Recorded = { step: string; version: string | null; bytes: number | null; via: string | null; error: string | null };

function steps(): Recorded[] {
  return tauri.invoke.mock.calls
    .filter(([command]) => command === "record_update")
    .map(([, args]) => args as unknown as Recorded);
}

function release(options: { download?: () => Promise<void>; install?: () => Promise<void> } = {}) {
  return {
    version: "0.1.21",
    download: vi.fn(async (progress: (event: unknown) => void) => {
      progress({ event: "Started", data: { contentLength: 100 } });
      progress({ event: "Progress", data: { chunkLength: 100 } });
      await options.download?.();
    }),
    install: vi.fn(options.install ?? (async () => undefined)),
  };
}

function onWindows(windows: boolean) {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(windows ? "Mozilla/5.0 (Windows NT 10.0)" : "Mozilla/5.0 (Macintosh)");
}

describe("the updater's steps in latency.jsonl", () => {
  beforeEach(() => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    tauri.check.mockReset();
    tauri.invoke.mockClear();
    tauri.closeHandler = null;
  });
  afterEach(() => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    vi.restoreAllMocks();
  });

  it("says a launch was up to date, or why it could not tell", async () => {
    tauri.check.mockResolvedValueOnce(null);
    expect(await downloadUpdateInBackground(() => undefined)).toBeNull();
    tauri.check.mockRejectedValueOnce(new Error("error sending request"));
    expect(await downloadUpdateInBackground(() => undefined)).toBeNull();

    expect(steps().map((recorded) => recorded.step)).toEqual(["up_to_date", "check_failed"]);
    expect(steps()[1].error).toBe("error sending request");
  });

  it("keeps how much of a download arrived when it broke off", async () => {
    onWindows(true);
    tauri.check.mockResolvedValueOnce(
      release({ download: async () => Promise.reject(new Error("connection reset")) }),
    );
    const report = vi.fn();
    expect(await downloadUpdateInBackground(report)).toBeNull();

    expect(steps().map((recorded) => recorded.step)).toEqual(["found", "download_failed"]);
    expect(steps()[1]).toMatchObject({ version: "0.1.21", bytes: 100, error: "connection reset" });
    expect(report).toHaveBeenLastCalledWith(null);
  });

  it("on Windows, writes the install down before the installer closes Ella, and says what asked", async () => {
    onWindows(true);
    const update = release();
    tauri.check.mockResolvedValueOnce(update);
    const apply = await downloadUpdateInBackground(() => undefined);
    expect(apply).not.toBeNull();
    expect(update.install).not.toHaveBeenCalled();

    await tauri.closeHandler?.();
    expect(update.install).toHaveBeenCalledOnce();
    expect(steps().map((recorded) => recorded.step)).toEqual(["found", "downloaded", "installing"]);
    expect(steps()[2]).toMatchObject({ version: "0.1.21", via: "close" });
    const installing = tauri.invoke.mock.invocationCallOrder[2];
    expect(installing).toBeLessThan(update.install.mock.invocationCallOrder[0]);
  });

  it("on Windows, keeps why an install failed and still lets the window close", async () => {
    onWindows(true);
    const update = release({ install: async () => Promise.reject(new Error("os error 32")) });
    tauri.check.mockResolvedValueOnce(update);
    const apply = await downloadUpdateInBackground(() => undefined);

    await expect(apply?.()).rejects.toThrow("os error 32");
    expect(steps().slice(-2)).toMatchObject([
      { step: "installing", via: "restart" },
      { step: "install_failed", via: "restart", error: "os error 32" },
    ]);
    // The window closing does not try a second time.
    await tauri.closeHandler?.();
    expect(update.install).toHaveBeenCalledOnce();
  });

  it("on a Mac, says the new version is in place for the next launch", async () => {
    onWindows(false);
    const update = release();
    tauri.check.mockResolvedValueOnce(update);
    const apply = await downloadUpdateInBackground(() => undefined);

    expect(update.install).toHaveBeenCalledOnce();
    expect(steps().map((recorded) => recorded.step)).toEqual(["found", "downloaded", "installed"]);
    await apply?.();
    expect(tauri.relaunch).toHaveBeenCalledOnce();
  });

  it("goes on with the update when the step cannot be written", async () => {
    onWindows(true);
    tauri.invoke.mockRejectedValue(new Error("no such command"));
    const update = release();
    tauri.check.mockResolvedValueOnce(update);
    const apply = await downloadUpdateInBackground(() => undefined);
    await apply?.();
    expect(update.install).toHaveBeenCalledOnce();
    tauri.invoke.mockResolvedValue(undefined);
  });
});

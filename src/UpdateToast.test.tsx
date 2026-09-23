import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import type { ApplyUpdate, UpdateProgress } from "./lib/updates";

// The real module talks to the Tauri updater; these tests drive its two
// outputs by hand: the progress it reports, and the apply function it returns.
const updater = vi.hoisted(() => ({
  report: null as ((progress: UpdateProgress | null) => void) | null,
  finish: null as ((apply: ApplyUpdate | null) => void) | null,
}));

vi.mock("./lib/updates", () => ({
  installExitsTheApp: () => false,
  downloadUpdateInBackground: (report: (progress: UpdateProgress | null) => void) => {
    updater.report = report;
    return new Promise<ApplyUpdate | null>((resolve) => {
      updater.finish = resolve;
    });
  },
}));

describe("background update", () => {
  beforeEach(() => window.localStorage.clear());

  it("shows progress in a toast while the app stays usable", async () => {
    render(<App />);
    const start = await screen.findByRole("button", { name: /let’s start/i });

    act(() =>
      updater.report?.({ stage: "downloading", version: "0.1.4", downloadedBytes: 50, totalBytes: 100 }),
    );
    expect(screen.getByText("Updating Ella")).toBeInTheDocument();
    expect(screen.getByText("Version 0.1.4 — 50%")).toBeInTheDocument();

    // The old flow took over the whole screen; now the learner carries on.
    fireEvent.click(start);
    expect(screen.getByLabelText("What should Ella call you?")).toBeInTheDocument();
    expect(screen.getByText("Updating Ella")).toBeInTheDocument();
  });

  it("offers a restart once the update is ready, and can be dismissed", async () => {
    const apply = vi.fn(async () => undefined);
    render(<App />);
    await screen.findByRole("button", { name: /let’s start/i });

    act(() => {
      updater.report?.({ stage: "ready", version: "0.1.4", downloadedBytes: 100, totalBytes: 100 });
      updater.finish?.(apply);
    });
    expect(await screen.findByText("Update ready")).toBeInTheDocument();
    expect(screen.getByText("Starts next time you open Ella")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Restart" }));
    expect(apply).toHaveBeenCalledOnce();

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("Update ready")).not.toBeInTheDocument();
  });

  it("shows nothing when there is no update", async () => {
    render(<App />);
    await screen.findByRole("button", { name: /let’s start/i });
    act(() => updater.finish?.(null));
    expect(screen.queryByText("Updating Ella")).not.toBeInTheDocument();
    expect(screen.queryByText("Update ready")).not.toBeInTheDocument();
  });
});

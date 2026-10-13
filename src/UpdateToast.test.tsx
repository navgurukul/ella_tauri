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

const shown = vi.hoisted(() => ({
  afterUpdate: null as null | { version: string; notes: string[] },
}));

vi.mock("./lib/whatsNew", () => ({
  whatsNewAfterUpdate: async () => shown.afterUpdate,
  markWhatsNewSeen: (version: string) => window.localStorage.setItem("ella-whats-new-seen", version),
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
  beforeEach(() => {
    window.localStorage.clear();
    shown.afterUpdate = null;
  });

  it("opens what the new version brings, in the learner's words, from its card", async () => {
    render(<App />);
    await screen.findByRole("button", { name: /let’s start/i });

    act(() =>
      updater.report?.({
        stage: "downloading",
        version: "0.1.21",
        downloadedBytes: 50,
        totalBytes: 100,
        notes: ["Ella starts answering a little sooner.", "Long answers are not lost any more."],
      }),
    );
    const open = screen.getByRole("button", { name: "What’s new" });
    expect(open).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Long answers are not lost any more.")).not.toBeInTheDocument();

    fireEvent.click(open);
    const list = screen.getByRole("list", { name: "What’s new in version 0.1.21" });
    expect(list).toHaveTextContent("Ella starts answering a little sooner.");
    expect(list).toHaveTextContent("Long answers are not lost any more.");

    fireEvent.click(screen.getByRole("button", { name: "Hide what’s new" }));
    expect(screen.queryByRole("list", { name: /what’s new/i })).not.toBeInTheDocument();
  });

  it("offers nothing to open when the release says nothing new", async () => {
    render(<App />);
    await screen.findByRole("button", { name: /let’s start/i });
    act(() => updater.report?.({ stage: "downloading", version: "0.1.21", downloadedBytes: 0, totalBytes: 0 }));
    expect(screen.getByText("Updating Ella")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "What’s new" })).not.toBeInTheDocument();
  });

  it("shows, once, what the version just installed brought", async () => {
    shown.afterUpdate = { version: "0.1.21", notes: ["On Windows, updates install more smoothly."] };
    render(<App />);
    expect(await screen.findByText("Ella is updated")).toBeInTheDocument();
    expect(screen.getByText("Version 0.1.21")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restart" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "What’s new" }));
    expect(screen.getByRole("list", { name: "What’s new in version 0.1.21" })).toHaveTextContent(
      "On Windows, updates install more smoothly.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("Ella is updated")).not.toBeInTheDocument();
    expect(window.localStorage.getItem("ella-whats-new-seen")).toBe("0.1.21");
  });

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

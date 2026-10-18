import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import MiniVideoTile, { hlsUrl } from "@/components/MiniVideoTile";

vi.mock("hls.js", () => {
  const stub = vi.fn().mockImplementation(() => ({
    loadSource: vi.fn(),
    attachMedia: vi.fn(),
  }));
  const hls = Object.assign(stub, {
    isSupported: () => true,
    Events: { ERROR: "hlsError" },
  });
  return { default: hls };
});

import HlsStub from "hls.js";

afterEach(cleanup);

describe("hlsUrl", () => {
  it("returns null without a venue template", () => {
    expect(hlsUrl("dock_cam")).toBeNull();
  });

  it("substitutes the camera id", () => {
    const tpl = "https://edge.local/hls/{camera}/index.m3u8";
    vi.stubEnv("NEXT_PUBLIC_SHADO_HLS_URL", tpl);
    expect(hlsUrl("dock cam/1")).toBe(
      "https://edge.local/hls/dock%20cam%2F1/index.m3u8",
    );
    vi.unstubAllEnvs();
  });
});

describe("MiniVideoTile", () => {
  it("withholds with an explicit note when no feed key is set", () => {
    vi.stubEnv("NEXT_PUBLIC_SHADO_HLS_URL", "");
    render(<MiniVideoTile cameraId="dock_cam" />);
    expect(screen.getByTestId("tile-withheld-dock_cam").textContent).toContain(
      "withheld",
    );
    expect(screen.queryByTestId("video-dock_cam")).toBeNull();
    vi.unstubAllEnvs();
  });

  it("attaches hls.js when a feed key exists", async () => {
    vi.stubEnv("NEXT_PUBLIC_SHADO_HLS_URL", "https://edge.local/hls/{camera}/index.m3u8");
    const loadSource = vi.fn();
    const attachMedia = vi.fn();
    (HlsStub as unknown as { mockImplementation: (fn: () => unknown) => void })
      .mockImplementation(function () {
        return { loadSource, attachMedia, destroy: vi.fn(), on: vi.fn() };
      });

    render(<MiniVideoTile cameraId="dock_cam" />);
    await vi.waitFor(() =>
      expect((HlsStub as unknown as { mock: { calls: unknown[] } }).mock.calls.length)
        .toBeGreaterThan(0),
    );
    expect(loadSource.mock.calls[0][0]).toBe("https://edge.local/hls/dock_cam/index.m3u8");
    expect(attachMedia).toHaveBeenCalledWith(expect.any(HTMLVideoElement));
    // Tile is at loading state until 'playing' fires — no fake live chip.
    expect(screen.getByTestId("tile-dock_cam").textContent).not.toContain("live");
    vi.unstubAllEnvs();
  });
});

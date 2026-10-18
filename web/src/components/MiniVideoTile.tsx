"use client";

import { useEffect, useRef, useState } from "react";
import type Hls from "hls.js";

/**
 * MiniVideoTile (docs/12 §3): one camera's HLS tile + a `<canvas>` overlay
 * slot for tracked boxes/counting lines.
 *
 * Feed keys come from the venue edge node (NEXT_PUBLIC_SHADO_HLS_URL, e.g.
 * `https://edge.local/hls/{camera}/index.m3u8`). When no template is set or
 * the stream errors, the tile shows an explicit withhold note — never a
 * silently-cycled still — per docs/12 accessibility rules.
 */
export function hlsUrl(cameraId: string): string | null {
  const tpl = process.env.NEXT_PUBLIC_SHADO_HLS_URL;
  return tpl ? tpl.replace("{camera}", encodeURIComponent(cameraId)) : null;
}

type TileState = "withheld" | "loading" | "live" | "error";

interface Props {
  cameraId: string;
  feedUrl?: string;
}

export default function MiniVideoTile({ cameraId, feedUrl }: Props) {
  const feedKey = feedUrl ?? hlsUrl(cameraId);
  const [state, setState] = useState<TileState>(feedKey ? "loading" : "withheld");
  const videoRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    const url = feedKey;
    if (!url) {
      setState("withheld");
      return;
    }
    const video = videoRef.current;
    if (!video) return;
    let hls: Hls | null = null;
    let tornDown = false;

    void import("hls.js").then(({ default: HlsCtor }) => {
      if (tornDown) return;
      if (!HlsCtor.isSupported()) {
        // Native HLS (Safari) fallback.
        video.src = url;
        video.play().catch(() => setState("error"));
        return;
      }
      hls = new HlsCtor({ capLevelToPlayerSize: true });
      hls.on(HlsCtor.Events.ERROR, (_ev, data) => {
        if (!data.fatal) return;
        setState("error");
        hls?.destroy();
      });
      hls.loadSource(url);
      hls.attachMedia(video);
    });

    const onPlaying = () => setState("live");
    video.addEventListener("playing", onPlaying, { once: true });

    return () => {
      tornDown = true;
      video.removeEventListener("playing", onPlaying);
      hls?.destroy?.();
    };
  }, [cameraId, feedKey]);

  return (
    <div className="rounded border border-zinc-800 bg-zinc-900 p-2" data-testid={`tile-${cameraId}`}>
      <p className="mb-1 font-mono text-[10px] uppercase tracking-wide text-zinc-400">
        {cameraId}
        {state === "live" ? " · live" : state === "error" ? " · feed offline" : ""}
      </p>
      {state === "withheld" ? (
        <p
          data-testid={`tile-withheld-${cameraId}`}
          className="py-6 text-center text-xs text-zinc-500"
        >
          feed withheld — no HLS source configured for this venue
        </p>
      ) : (
        <video
          ref={videoRef}
          data-testid={`video-${cameraId}`}
          className="block aspect-video w-full rounded bg-zinc-950"
          muted
          playsInline
          autoPlay
        />
      )}
      <canvas className="mt-1 aspect-video w-full" data-testid={`overlay-${cameraId}`} />
    </div>
  );
}

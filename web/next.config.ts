import type { NextConfig } from "next";

// Proxy REST to the Rust api-server in dev (the browser sees same-origin,
// so CORS stays single-origin per docs/10 §2). WS connects directly.
const backend = process.env.SHADO_API_URL ?? "http://localhost:8080";

const nextConfig: NextConfig = {
  async rewrites() {
    return [
      { source: "/api/v1/:path*", destination: `${backend}/api/v1/:path*` },
      { source: "/healthz", destination: `${backend}/healthz` },
    ];
  },
};

export default nextConfig;

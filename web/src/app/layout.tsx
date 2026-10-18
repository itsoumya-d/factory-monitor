import type { Metadata } from "next";
import "./globals.css";
import { QueryProvider } from "@/components/QueryProvider";
import AppShell from "@/components/AppShell";

export const metadata: Metadata = {
  title: "Shado — multimodal monitoring",
  description: "Live ops, corrections and RL knobs for the Shado sensing stack.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className="dark">
      <body className="antialiased">
        <QueryProvider>
          <AppShell>{children}</AppShell>
        </QueryProvider>
      </body>
    </html>
  );
}

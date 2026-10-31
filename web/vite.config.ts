import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The engine serves the built app from its own address, so asset paths are relative to it.
export default defineConfig({
  plugins: [react()],
  base: "/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Fonts ship inside the app; nothing is fetched from the internet.
    assetsInlineLimit: 0,
  },
  server: {
    port: 5173,
    strictPort: true,
  },
});

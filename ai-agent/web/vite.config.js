import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The FastAPI backend has no CORS middleware, so the browser talks to Vite and
// Vite forwards /api/* to 127.0.0.1:8000.  Both dev and preview use the proxy.
const proxy = {
  "/api": {
    target: process.env.VITE_PROXY_TARGET || "http://127.0.0.1:8000",
    changeOrigin: true,
    rewrite: (path) => path.replace(/^\/api/, ""),
  },
};

export default defineConfig({
  plugins: [react()],
  server: { host: "127.0.0.1", port: 5173, strictPort: true, proxy },
  preview: { host: "127.0.0.1", port: 4173, strictPort: true, proxy },
});

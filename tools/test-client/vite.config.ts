import { defineConfig } from "vite";

export default defineConfig({
  server: {
    port: 5173,
    open: true,
    proxy: {
      "/signal": {
        target: "http://localhost:8450",
        ws: true,
      },
      "/instances": {
        target: "http://localhost:8450",
      },
      "/health": {
        target: "http://localhost:8450",
      },
    },
  },
});

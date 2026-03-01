import { spawn, ChildProcess } from "node:child_process";

export interface VScreenServer {
  process: ChildProcess;
  port: number;
  baseUrl: string;
  wsUrl: (instanceId: string) => string;
  stop: () => Promise<void>;
}

/**
 * Start a vscreen server for integration testing.
 */
export async function startServer(port = 0): Promise<VScreenServer> {
  const actualPort = port || (await findFreePort());

  const proc = spawn("cargo", ["run", "--", "--listen", `127.0.0.1:${actualPort}`], {
    cwd: process.cwd(),
    stdio: ["pipe", "pipe", "pipe"],
    env: { ...process.env, RUST_LOG: "info" },
  });

  // Wait for server to be ready
  await new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("Server start timeout")), 10000);
    proc.stderr?.on("data", (data: Buffer) => {
      const text = data.toString();
      if (text.includes("server listening")) {
        clearTimeout(timeout);
        resolve();
      }
    });
    proc.on("error", (err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });

  return {
    process: proc,
    port: actualPort,
    baseUrl: `http://127.0.0.1:${actualPort}`,
    wsUrl: (id: string) => `ws://127.0.0.1:${actualPort}/signal/${id}`,
    stop: async () => {
      proc.kill("SIGTERM");
      await new Promise<void>((resolve) => proc.on("close", resolve));
    },
  };
}

async function findFreePort(): Promise<number> {
  const { createServer } = await import("node:net");
  return new Promise((resolve) => {
    const server = createServer();
    server.listen(0, () => {
      const addr = server.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      server.close(() => resolve(port));
    });
  });
}

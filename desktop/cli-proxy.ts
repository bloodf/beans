import { connect } from "node:net";
import type { Plugin } from "vite";
import { spawn } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { randomBytes } from "node:crypto";


/** Never exposes the CLI credential to the page or accepts an anonymous proxy upgrade. */
export function cliProxy(): Plugin {
  return {
    name: "beans-local-cli",
    apply: "serve",
    configureServer(server) {
      let broker: ChildProcess | null = null;
      let brokerReady: Promise<void> | null = null;
      let brokerSecret = "";
      let brokerPort = 0;
      const prepareBroker = (): Promise<void> => {
        brokerReady ??= new Promise<void>((resolve, reject) => {
          brokerSecret = randomBytes(32).toString("hex");
          broker = spawn("go", ["run", ".", "--cli-dev-proxy"], { cwd: import.meta.dirname, env: { ...process.env, BEANS_CLI_DEV_PROXY: "1", GOPROXY: "off", GOSUMDB: "off", GOTOOLCHAIN: "local" }, stdio: ["pipe", "pipe", "pipe"] });
          broker.stdin?.write(`${brokerSecret}\n`);
          broker.stderr?.resume();
          const timer = setTimeout(() => { broker?.kill(); reject(new Error("Native CLI proxy startup timed out")); }, 60000);
          let output = "";
          broker.stdout?.on("data", (data: Buffer) => {
            output += data.toString();
            const ready = /^beans-cli-proxy-ready ([0-9]+)\r?$/m.exec(output);
            if (ready) { brokerPort = Number(ready[1]); clearTimeout(timer); resolve(); }
          });
          broker.on("error", () => { clearTimeout(timer); brokerReady = null; reject(new Error("Native CLI proxy startup failed")); });
          broker.on("exit", () => { clearTimeout(timer); brokerReady = null; reject(new Error("Native CLI proxy exited")); });
        });
        return brokerReady;
      };
      server.httpServer?.once("close", () => { broker?.stdin?.end(); broker?.kill(); });
      server.httpServer?.on("upgrade", (request, socket, head) => {
        if (request.url !== "/__beans_cli_ws") return;
        const reject = () => { socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n"); };
        const peer = request.socket.remoteAddress;
        const local = peer === "127.0.0.1" || peer === "::1" || peer === "::ffff:127.0.0.1";
        const port = request.socket.localPort;
        const host = request.headers.host;
        const one = (name: string) => request.rawHeaders.filter((_, index) => index % 2 === 0 && request.rawHeaders[index]!.toLowerCase() === name).length === 1;
        if (!local || !one("host") || !one("origin") || !host || ![`localhost:${port}`, `127.0.0.1:${port}`].includes(host)
          || request.headers.origin !== `http://${host}` || request.headers.authorization !== undefined || request.headers["beans-dev-proxy"] !== undefined) { reject(); return; }
          void prepareBroker().then(() => {
          const host = connect({ host: "127.0.0.1", port: brokerPort });
          const stop = () => { host.destroy(); socket.destroy(); };
          host.on("error", stop);
          socket.on("error", stop);
          socket.on("close", () => host.destroy());
          host.on("connect", () => {
            const lines = ["GET /__beans_cli_ws HTTP/1.1", `Beans-Dev-Proxy: ${brokerSecret}`];
            for (let index = 0; index < request.rawHeaders.length; index += 2) lines.push(`${request.rawHeaders[index]}: ${request.rawHeaders[index + 1]}`);
            host.write(lines.join("\r\n") + "\r\n\r\n");
            if (head.length) host.write(head);
            socket.pipe(host).pipe(socket);
          });
          }).catch(() => reject());
      });
    },
  };
}

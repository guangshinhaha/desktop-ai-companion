import http from "node:http";
import { createChatHandler } from "../src/chat.ts";
import { createModelAdapter } from "../src/adapter.ts";
import { loadConfig } from "../src/config.ts";
import { stubRoute } from "../src/route.ts";

export const CONFIG = { route: { kind: "stub" }, ceiling: "official-closed", models: ["stub-small", "stub-large"] };

/** Start the handler on loopback only. Returns post(), the captured log events and the route's call log. */
export async function start({ config = CONFIG, route = stubRoute(), options, logger } = {}) {
  const cfg = loadConfig(config);
  const calls = [];
  const spy = { name: route.name, stream: (r) => { calls.push(r); return route.stream(r); } };
  const events = [];
  const handler = createChatHandler(cfg, createModelAdapter(cfg, spy), logger ?? ((e) => events.push(e)), options);
  const server = http.createServer((req, res) => { handler(req, res); });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  server.unref(); // a failed assertion must not leave the runner hanging
  const { port } = server.address();
  const url = `http://127.0.0.1:${port}/chat`;
  async function post(body, { raw = false } = {}) {
    const res = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: raw ? body : JSON.stringify(body) });
    return { status: res.status, text: await res.text() };
  }
  return { post, url, port, events, calls, close: () => new Promise((r) => { server.closeAllConnections(); server.close(r); }) };
}

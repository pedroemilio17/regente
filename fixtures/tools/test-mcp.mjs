#!/usr/bin/env node
// Minimal stdio MCP server used only to record CLI fixtures (M0).
// Exposes one tool, `ping`, which echoes its `text` argument.
import { createInterface } from "node:readline";

const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");

createInterface({ input: process.stdin }).on("line", (line) => {
  let req;
  try {
    req = JSON.parse(line);
  } catch {
    return;
  }
  if (req.id === undefined) return; // notification
  const reply = (result) => send({ jsonrpc: "2.0", id: req.id, result });
  switch (req.method) {
    case "initialize":
      return reply({
        protocolVersion: req.params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: {} },
        serverInfo: { name: "regente_test", version: "0.0.1" },
      });
    case "tools/list":
      return reply({
        tools: [
          {
            name: "ping",
            description: "Echo test tool. Returns pong plus the given text.",
            inputSchema: {
              type: "object",
              properties: { text: { type: "string" } },
              required: ["text"],
            },
          },
        ],
      });
    case "tools/call":
      return reply({
        content: [{ type: "text", text: `pong: ${req.params?.arguments?.text ?? ""}` }],
      });
    case "ping":
      return reply({});
    default:
      return send({ jsonrpc: "2.0", id: req.id, error: { code: -32601, message: "method not found" } });
  }
});

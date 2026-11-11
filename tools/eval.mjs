// Evaluates an expression in the first Pane bar page: node eval.mjs "<js expression>"
const pages = (await (await fetch("http://127.0.0.1:9333/json")).json()).filter((p) => p.type === "page");
const page = pages.find((p) => !p.title.includes("overflow")) ?? pages[0];
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: process.argv[2], awaitPromise: true } }));
const msg = await new Promise((r) => (ws.onmessage = (e) => r(JSON.parse(e.data))));
console.log(msg.result?.result?.value);
ws.close();

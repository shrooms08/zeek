// Checks that a Zeek live stream answers: opens the WebSocket (https:// is
// turned into wss://) and waits for the board event the engine sends first.
//   node scripts/ws-check.mjs https://playzeek.pages.dev/api/v1/stream
const target = process.argv[2];
if (target === undefined) {
  console.error("usage: ws-check.mjs <stream url>");
  process.exit(2);
}
const url = target.replace(/^http/, "ws");
const socket = new WebSocket(url);
const timer = setTimeout(() => {
  console.error(`no board event from ${url} within 20 s`);
  process.exit(1);
}, 20_000);
socket.addEventListener("message", (event) => {
  const message = JSON.parse(String(event.data));
  if (message.event === "board") {
    clearTimeout(timer);
    console.log(`live stream OK: board for round ${message.round}`);
    socket.close();
    process.exit(0);
  }
});
socket.addEventListener("error", () => {
  clearTimeout(timer);
  console.error(`could not open ${url}`);
  process.exit(1);
});

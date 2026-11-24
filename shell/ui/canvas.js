// Escape closes the canvas. Chat is in chat.js.
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") window.__TAURI__.core.invoke("close_canvas");
});

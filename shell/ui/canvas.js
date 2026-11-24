// The canvas is one input. Escape closes it. Chat is #31, so Enter does nothing yet.
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") window.__TAURI__.core.invoke("close_canvas");
});

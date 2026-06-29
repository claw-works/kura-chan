import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

let httpBase = "http://127.0.0.1:18099";
let gender = "girl";
let subtitle = "";

function el(sel: string): HTMLElement {
  return document.querySelector(sel)!;
}
function setStatus(s: string) {
  el("#status").textContent = s;
}
function setBubble(s: string) {
  el("#bubble").textContent = s;
}
function loadPortrait() {
  const img = el("#portrait") as HTMLImageElement;
  // cache-bust so appearance changes (outfit/scene via [do:]) refresh
  img.src = `${httpBase}/assets/composite_png/${gender}?h=480&t=${Date.now()}`;
}

async function init() {
  try {
    const cfg = await invoke<{ httpBase: string }>("get_config");
    if (cfg?.httpBase) httpBase = cfg.httpBase;
  } catch {
    /* keep default */
  }
  loadPortrait();

  // server → frontend events (forwarded from the WS client in Rust)
  await listen<string>("ws-status", (e) => setStatus(String(e.payload)));
  await listen<any>("subtitle", (e) => {
    // reply text streams in per sentence; empty `final` marker just ends the turn
    subtitle += e.payload?.text ?? "";
    if (subtitle) setBubble(subtitle);
  });
  await listen<any>("sync", (e) => {
    const g = e.payload?.gender;
    if (g && g !== gender) {
      gender = g;
      loadPortrait();
    }
  });

  const form = el("#chat-form") as HTMLFormElement;
  form.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const input = el("#chat-input") as HTMLInputElement;
    const text = input.value.trim();
    if (!text) return;
    subtitle = "";
    setBubble("…");
    input.value = "";
    try {
      await invoke("send_text", { text });
    } catch (err) {
      setBubble("发送失败：" + err);
    }
  });
}

window.addEventListener("DOMContentLoaded", init);

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

function label(s: string): string {
  if (s === "connected") return "已连接";
  if (s === "connecting") return "连接中…";
  if (s === "closed") return "连接已关闭，重连中…";
  if (s.startsWith("disconnected")) return "已断开，重连中…";
  return s;
}

async function init() {
  // subscribe FIRST so we don't miss a "connected" emitted during connect
  await listen<string>("ws-status", (e) => setStatus(label(String(e.payload))));
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

  // then read current config + status (covers events emitted before we subscribed)
  try {
    const cfg = await invoke<{ httpBase: string; status: string }>("get_config");
    if (cfg?.httpBase) httpBase = cfg.httpBase;
    if (cfg?.status) setStatus(label(cfg.status));
  } catch {
    /* keep defaults */
  }
  loadPortrait();

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

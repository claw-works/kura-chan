import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

let httpBase = "http://127.0.0.1:18099";
let gender = "girl";
let appearance: Record<string, any> = {};
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

function label(s: string): string {
  if (s === "connected") return "已连接";
  if (s === "connecting") return "连接中…";
  if (s === "closed") return "连接已关闭，重连中…";
  if (s.startsWith("disconnected")) return "已断开，重连中…";
  return s;
}

// Appearance values may be a full filename ("40_costume_pajamas_pink.png") or a
// bare variant ("pajamas_pink"); composite_png wants the variant.
function variant(val: any, slot: string): string {
  if (typeof val !== "string" || !val) return "";
  let v = val.replace(/\.(png|webp|jpe?g)$/i, "");
  const i = v.indexOf(slot + "_");
  if (i >= 0) v = v.slice(i + slot.length + 1);
  return v;
}

function loadPortrait() {
  const img = el("#portrait") as HTMLImageElement;
  const a = appearance || {};
  const p = new URLSearchParams();
  p.set("h", "480");
  // Use the shared actor's appearance from the server; fall back to a dressed
  // default so we never render the bare (bald/naked) base body.
  const hairBack = variant(a.hair_back, "hair_back") || "short_black";
  // front hair matches back hair (front/back halves of the same hairstyle);
  // fall back to the back-hair variant so the fringe is never missing.
  const hairFront = variant(a.hair_front, "hair_front") || hairBack;
  const costume = variant(a.costume, "costume") || "jacket";
  p.set("hair_back", hairBack);
  p.set("hair_front", hairFront);
  p.set("costume", costume);
  if (a.blush === true || (typeof a.blush === "string" && a.blush)) {
    p.set("blush", typeof a.blush === "string" ? variant(a.blush, "blush") || "faint" : "faint");
  }
  if (a.glasses === true) p.set("glasses", "1");
  p.set("t", String(Date.now())); // cache-bust on appearance change
  img.src = `${httpBase}/assets/composite_png/${gender}?${p.toString()}`;
}

async function init() {
  // subscribe FIRST so we don't miss events emitted during connect
  await listen<string>("ws-status", (e) => setStatus(label(String(e.payload))));
  await listen<any>("subtitle", (e) => {
    subtitle += e.payload?.text ?? "";
    if (subtitle) setBubble(subtitle);
  });
  await listen<any>("sync", (e) => {
    const p = e.payload;
    if (p?.gender) gender = p.gender;
    if (p?.appearance) appearance = p.appearance; // full appearance on connect
    loadPortrait();
  });

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

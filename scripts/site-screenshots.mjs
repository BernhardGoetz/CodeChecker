#!/usr/bin/env node
// Nimmt die Screenshots der Website neu auf (site/img/*.webp) und erzeugt das
// Vorschaubild für soziale Netzwerke (site/img/og-image.png).
//
//   node scripts/site-screenshots.mjs
//
// Startet die App-Oberfläche im Browser mit dem Mock-Backend
// (src/dev/mockBackend.ts, wie `VITE_MOCK=1 npm run dev`) und steuert sie mit
// Playwright. Playwright ist keine Abhängigkeit des Projekts; es muss global
// installiert sein (`require("playwright")`, Chromium inklusive). Die Bilder
// enthalten nur die Beispieldaten des Mock-Backends, keine persönlichen Daten.
import { createRequire } from "node:module";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const IMG = path.join(ROOT, "site/img");
const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const VIEWPORT = { width: 1280, height: 800 };
const WEBP_QUALITY = 0.8;

async function startDevServer() {
  process.env.VITE_MOCK = "1";
  const { createServer } = await import("vite");
  const server = await createServer({
    root: ROOT,
    logLevel: "warn",
    server: { port: 1430, strictPort: false, host: "127.0.0.1" },
  });
  await server.listen();
  const url = server.resolvedUrls?.local?.[0];
  if (!url) throw new Error("Vite-Server ohne URL");
  return { server, url };
}

/** PNG -> WebP mit dem Encoder von Chromium (keine weiteren Werkzeuge). */
async function toWebp(page, png) {
  const b64 = await page.evaluate(
    async ({ data, quality }) => {
      const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      canvas.getContext("2d").drawImage(bitmap, 0, 0);
      const blob = await canvas.convertToBlob({ type: "image/webp", quality });
      const buf = new Uint8Array(await blob.arrayBuffer());
      let s = "";
      for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode(...buf.subarray(i, i + 0x8000));
      return btoa(s);
    },
    { data: png.toString("base64"), quality: WEBP_QUALITY },
  );
  return Buffer.from(b64, "base64");
}

async function settle(page) {
  await page.evaluate(() => {
    document.activeElement?.blur?.();
    window.scrollTo(0, 0);
  });
  await page.mouse.move(0, VIEWPORT.height - 1);
  await page.waitForTimeout(400);
}

async function capture(browser, appUrl, scheme) {
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2, colorScheme: scheme, locale: "de-DE" });
  await ctx.addInitScript(() => {
    try {
      localStorage.setItem("code-pruefer.editorHeight", "200");
      localStorage.removeItem("code-pruefer.theme");
      localStorage.removeItem("code-pruefer.tab");
    } catch {
      /* ohne Speicher gilt die Voreinstellung */
    }
  });
  const page = await ctx.newPage();
  await page.goto(`${appUrl}?update=none`);
  await page.getByRole("button", { name: "Prüfen", exact: true }).waitFor();
  await page.waitForTimeout(1500);

  const shots = {};
  // 1. Aufgabe: Aufgabenblatt hochladen (Mock: Bild mit Texterkennung), Text als geprüft markieren.
  await page.getByRole("tab", { name: /Aufgabe/ }).click();
  await page.getByRole("button", { name: "Aufgabe hochladen …" }).click();
  await page.getByLabel(/Text geprüft/).check();
  await settle(page);
  shots.aufgabe = await page.screenshot();

  // 2. Testfälle mit KI-Panel.
  await page.getByRole("tab", { name: /Testfälle/ }).click();
  await page.getByRole("button", { name: "Testfälle mit KI erzeugen" }).click();
  await page.getByText(/Konfidenz: Grün/).waitFor();
  await settle(page);
  shots.testfaelle = await page.screenshot();

  // 3. Ergebnis (mit Variationstest).
  await page.getByRole("button", { name: "Prüfen", exact: true }).click();
  await page.getByText(/Variationstest bestanden/).waitFor();
  await settle(page);
  shots.ergebnis = await page.screenshot();

  const suffix = scheme === "dark" ? "dunkel" : "hell";
  const out = {};
  for (const [name, png] of Object.entries(shots)) {
    const webp = await toWebp(page, png);
    const file = path.join(IMG, `${name}-${suffix}.webp`);
    await writeFile(file, webp);
    out[name] = png;
    console.log(`${path.relative(ROOT, file)}: ${(webp.length / 1024).toFixed(0)} KB`);
  }
  await ctx.close();
  return out;
}

async function ogImage(browser, ergebnisPng) {
  const icon = (await readFile(path.join(ROOT, "src-tauri/icons/128x128@2x.png"))).toString("base64");
  const shot = ergebnisPng.toString("base64");
  const ctx = await browser.newContext({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  await page.setContent(`<!doctype html><html lang="de"><head><meta charset="utf-8"><style>
    body { margin: 0; width: 1200px; height: 630px; overflow: hidden; background: #f6f8fa;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; color: #1f2328; }
    .text { position: absolute; left: 72px; top: 96px; width: 470px; }
    .brand { display: flex; align-items: center; gap: 18px; font-size: 40px; font-weight: 600; margin-bottom: 36px; }
    .brand img { width: 72px; height: 72px; }
    h1 { font-size: 46px; line-height: 1.2; margin: 0 0 28px; font-weight: 600; }
    p { font-size: 26px; color: #59636e; margin: 0; }
    .shot { position: absolute; left: 600px; top: 90px; width: 760px; border: 1px solid #d1d9e0;
      border-radius: 10px; box-shadow: 0 8px 24px rgba(31, 35, 40, 0.12); }
  </style></head><body>
    <div class="text">
      <div class="brand"><img src="data:image/png;base64,${icon}" alt="">Code-Prüfer</div>
      <h1>Programmieraufgaben lokal und sicher prüfen</h1>
      <p>Python und Java · kostenlos · Open Source · alles lokal</p>
    </div>
    <img class="shot" src="data:image/png;base64,${shot}" alt="">
  </body></html>`);
  await page.waitForTimeout(300);
  const png = await page.screenshot({ type: "png" });
  const file = path.join(IMG, "og-image.png");
  await writeFile(file, png);
  console.log(`${path.relative(ROOT, file)}: ${(png.length / 1024).toFixed(0)} KB`);
  await ctx.close();
}

const { server, url } = await startDevServer();
const browser = await chromium.launch();
try {
  const light = await capture(browser, url, "light");
  await capture(browser, url, "dark");
  await ogImage(browser, light.ergebnis);
} finally {
  await browser.close();
  await server.close();
}

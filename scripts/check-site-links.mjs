#!/usr/bin/env node
// Prüft die gebaute Website (site-dist/, siehe scripts/build-site.mjs):
//
//   node scripts/check-site-links.mjs
//
// - Jeder interne Link (relativ oder unter /CodeChecker/) zeigt auf eine
//   vorhandene Datei; ein Anker (#...) muss auf der Zielseite als id stehen.
// - Eingebundene Ressourcen (Stylesheets, Skripte, Bilder, Icons) liegen alle
//   auf der Website selbst: keine externen Ressourcen (Datenschutz).
// - Jede Seite trägt die Content-Security-Policy aus build-site.mjs und
//   enthält keine Inline-Styles, <style>-Blöcke oder Inline-Skripte.
// Externe Links (<a href="https://...">) werden nicht abgerufen.
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { CSP } from "./build-site.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUT = path.join(ROOT, "site-dist");
const SITE_PATH = "/CodeChecker/";

async function walk(dir) {
  const files = [];
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) files.push(...(await walk(p)));
    else files.push(p);
  }
  return files;
}

const decode = (s) => s.replace(/&amp;/g, "&").replace(/&quot;/g, '"').replace(/&#39;/g, "'");

function idsOf(html) {
  const ids = new Set();
  for (const m of html.matchAll(/\sid="([^"]*)"/g)) ids.add(decode(m[1]));
  return ids;
}

async function resolveTarget(fromFile, url) {
  let p = url.pathname;
  let file;
  if (p.startsWith(SITE_PATH)) file = path.join(OUT, decodeURIComponent(p.slice(SITE_PATH.length)));
  else return null;
  const st = await stat(file).catch(() => null);
  if (st?.isDirectory()) file = path.join(file, "index.html");
  else if (!st) return { file, exists: false };
  const exists = Boolean(await stat(file).catch(() => null));
  return { file, exists };
}

const files = await walk(OUT);
const htmlFiles = files.filter((f) => f.endsWith(".html"));
const idCache = new Map();
const errors = [];
let checked = 0;

for (const file of htmlFiles) {
  const html = await readFile(file, "utf8");
  const rel = path.relative(OUT, file).split(path.sep).join("/");
  // Basis-URL der Seite, als läge site-dist unter /CodeChecker/.
  const base = new URL(rel, `https://example.invalid${SITE_PATH}`);

  // Content-Security-Policy (siehe build-site.mjs) und nichts, was sie
  // blockieren würde: keine Inline-Styles, <style>-Blöcke, Inline-Skripte
  // oder Event-Handler-Attribute.
  const csp = [...html.matchAll(/<meta http-equiv="Content-Security-Policy" content="([^"]*)">/g)];
  if (csp.length !== 1 || decode(csp[0][1]) !== CSP) errors.push(`${rel}: Content-Security-Policy fehlt oder weicht ab`);
  if (/<[a-z][^>]*\sstyle\s*=/i.test(html)) errors.push(`${rel}: Inline-Style (style=...)`);
  if (/<style[\s>]/i.test(html)) errors.push(`${rel}: <style>-Block`);
  if (/<[a-z][^>]*\son[a-z]+\s*=/i.test(html)) errors.push(`${rel}: Event-Handler-Attribut (on...=)`);
  for (const m of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    if (!/\ssrc="/i.test(m[1]) || m[2].trim()) errors.push(`${rel}: Inline-Skript`);
  }

  const refs = [];
  for (const m of html.matchAll(/<(a|link|script|img|source)\b([^>]*)>/gi)) {
    const tag = m[1].toLowerCase();
    const attrs = m[2];
    const get = (name) => {
      const a = attrs.match(new RegExp(`\\s${name}="([^"]*)"`, "i"));
      return a ? decode(a[1]) : null;
    };
    if (tag === "a") {
      const href = get("href");
      if (href !== null) refs.push({ tag, value: href, embed: false });
    } else if (tag === "link") {
      const href = get("href");
      const relAttr = (get("rel") ?? "").toLowerCase();
      // canonical/alternate sind absolute Adressen der veröffentlichten Seite.
      if (href !== null) refs.push({ tag, value: href, embed: !/canonical|alternate/.test(relAttr) });
    } else if (tag === "script") {
      const src = get("src");
      if (src !== null) refs.push({ tag, value: src, embed: true });
    } else {
      const src = get("src");
      if (src !== null) refs.push({ tag, value: src, embed: true });
      const srcset = get("srcset");
      if (srcset) for (const part of srcset.split(",")) refs.push({ tag, value: part.trim().split(/\s+/)[0], embed: true });
    }
  }
  for (const m of html.matchAll(/<meta\s+(?:property|name)="(?:og:image|twitter:image)"\s+content="([^"]*)"/g)) {
    refs.push({ tag: "meta", value: decode(m[1]), embed: false, own: true });
  }

  for (const ref of refs) {
    checked++;
    const v = ref.value;
    if (/^(mailto|tel):/i.test(v)) continue;
    let url;
    // Absolute Adressen der eigenen Website wie interne behandeln.
    const own = v.startsWith("https://bernhardgoetz.github.io/CodeChecker/");
    try {
      url = own ? new URL(v.replace("https://bernhardgoetz.github.io", "https://example.invalid")) : new URL(v, base);
    } catch {
      errors.push(`${rel}: ungültige Adresse ${v}`);
      continue;
    }
    if (url.origin !== "https://example.invalid") {
      if (ref.embed) errors.push(`${rel}: externe Ressource eingebunden (<${ref.tag}>): ${v}`);
      else if (url.protocol !== "https:") errors.push(`${rel}: Link ohne HTTPS: ${v}`);
      continue;
    }
    const target = await resolveTarget(file, url);
    if (!target) {
      errors.push(`${rel}: Link außerhalb der Website: ${v}`);
      continue;
    }
    if (!target.exists) {
      errors.push(`${rel}: Ziel fehlt: ${v}`);
      continue;
    }
    if (url.hash && url.hash !== "#" && target.file.endsWith(".html")) {
      if (!idCache.has(target.file)) idCache.set(target.file, idsOf(await readFile(target.file, "utf8")));
      const id = decodeURIComponent(url.hash.slice(1));
      if (!idCache.get(target.file).has(id)) errors.push(`${rel}: Anker fehlt: ${v}`);
    }
  }
}

if (errors.length) {
  console.error(`${errors.length} Fehler:\n${errors.join("\n")}`);
  process.exit(1);
}
console.log(`Links geprüft: ${htmlFiles.length} Seiten, ${checked} Verweise, keine Fehler.`);

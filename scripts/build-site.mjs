#!/usr/bin/env node
// Baut die Website (GitHub Pages) nach site-dist/.
//
//   npm run build:site
//
// - Seiten aus site/ (HTML-Fragmente mit Kopfkommentar <!-- page: {...} -->)
//   bekommen Kopf, Navigation und Fußzeile; Platzhalter wie {{version}} oder
//   {{dl.windows}} werden ersetzt (unbekannte Platzhalter sind ein Fehler).
// - Die Doku-Unterseiten entstehen aus den Markdown-Dateien des Repositorys
//   (README.md, CONTRIBUTING.md, docs/...). Relative Links zeigen danach auf
//   die passende Unterseite oder auf die Datei bei GitHub. Ein Link auf eine
//   Datei, die es nicht gibt, bricht den Build ab.
// - Version und Download-Links kommen aus src-tauri/tauri.conf.json.
// - Dazu sitemap.xml, robots.txt, OpenGraph-/Twitter-Tags, App-Icon.
//
// Keine externen Ressourcen: alles liegt unter site-dist/.
import { cp, mkdir, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { existsSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Marked, Renderer } from "marked";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "site");
const OUT = path.join(ROOT, "site-dist");

export const SITE_URL = "https://bernhardgoetz.github.io/CodeChecker/";
const SITE_PATH = new URL(SITE_URL).pathname; // "/CodeChecker/"
const REPO = "https://github.com/BernhardGoetz/CodeChecker";
const OG_IMAGE = "img/og-image.png";

// Content-Security-Policy jeder Seite (als <meta>, GitHub Pages setzt keine
// eigenen Header). Alles kommt von der Website selbst; es gibt keine
// Inline-Styles, <style>-Blöcke, Inline-Skripte oder data:-Bilder, deshalb
// weder 'unsafe-inline' noch data:. frame-ancestors wirkt in <meta> nicht
// und fehlt deshalb. scripts/check-site-links.mjs prüft, dass jede Seite
// diese Richtlinie trägt und nichts Inline enthält.
export const CSP = [
  "default-src 'none'",
  "img-src 'self'",
  "style-src 'self'",
  "script-src 'self'",
  "font-src 'self'",
  "base-uri 'none'",
  "form-action 'none'",
  "object-src 'none'",
].join("; ");

// Doku-Unterseiten: Quelle im Repository -> Pfad auf der Website.
const DOCS = [
  {
    src: "README.md",
    slug: "doku/anleitung/",
    title: "Anleitung",
    description: "Benutzerhandbuch des Code-Prüfers: Installation, erster Start, Bedienung, häufige Probleme und Datenschutz.",
    lang: "de",
  },
  {
    src: "docs/WARNUNG_BEIM_INSTALLIEREN.md",
    slug: "doku/warnung-beim-installieren/",
    title: "Warnung beim Installieren",
    description: "Warum Windows bzw. macOS beim Installieren warnt, wie du die Warnung sicher umgehst und die Echtheit der Datei prüfst.",
    lang: "de",
  },
  {
    src: "docs/Privacy Policy.md",
    slug: "doku/datenschutz/",
    title: "Privacy policy",
    description: "Verbindliche Datenschutzerklärung des Programms (englisch): welche Verbindungen es gibt und welche Daten nie den Computer verlassen.",
    lang: "en",
  },
  {
    src: "docs/CODE_SIGNING_POLICY.md",
    slug: "doku/code-signing-policy/",
    title: "Code signing policy",
    description: "Was signiert wird, wer welche Rolle hat und wie ein Release entsteht (englisch).",
    lang: "en",
  },
  {
    src: "CONTRIBUTING.md",
    slug: "doku/mitmachen/",
    title: "Mitmachen",
    description: "Für Entwicklerinnen und Entwickler: Aufbau des Programms, selbst bauen, Tests, Release-Ablauf und Beitragsregeln.",
    lang: "de",
  },
];

const UI = {
  de: {
    skip: "Zum Inhalt springen",
    mainNav: "Hauptnavigation",
    nav: [
      { key: "start", label: "Start", href: "" },
      { key: "download", label: "Download", href: "#download" },
      { key: "faq", label: "Fragen", href: "#faq" },
      { key: "doku", label: "Doku", href: "doku/" },
      { key: "kontakt", label: "Kontakt", href: "kontakt/" },
      { key: "en", label: "English", href: "en/", lang: "en" },
    ],
    footer: [
      { label: "Anleitung", href: "doku/anleitung/" },
      { label: "Alle Dokumente", href: "doku/" },
      { label: "Kontakt & Impressum", href: "kontakt/" },
      { label: "Quellcode auf GitHub", href: `${REPO}` },
      { label: "English", href: "en/", lang: "en" },
    ],
    footerLine: (v) => `Code-Prüfer ${v} · <a href="${REPO}/blob/main/LICENSE">MIT-Lizenz</a> · privates, nicht kommerzielles Open-Source-Projekt`,
    locale: "de_DE",
    ogAlt: "Code-Prüfer: Programmieraufgaben lokal und sicher prüfen",
  },
  en: {
    skip: "Skip to content",
    mainNav: "Main navigation",
    nav: [
      { key: "en", label: "Home", href: "en/" },
      { key: "download", label: "Download", href: "en/#download" },
      { key: "doku", label: "Manual (German)", href: "doku/anleitung/", hreflang: "de" },
      { key: "start", label: "Deutsch", href: "", lang: "de" },
    ],
    footer: [
      { label: "Privacy policy", href: "doku/datenschutz/" },
      { label: "Code signing policy", href: "doku/code-signing-policy/" },
      { label: "Contact (German)", href: "kontakt/", hreflang: "de" },
      { label: "Source code on GitHub", href: `${REPO}` },
    ],
    footerLine: (v) => `Code-Prüfer ${v} · <a href="${REPO}/blob/main/LICENSE">MIT License</a> · private, non-commercial open-source project`,
    locale: "en_US",
    ogAlt: "Code-Prüfer: check programming exercises locally",
  },
};

const escapeHtml = (s) =>
  String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const decodeEntities = (s) =>
  s
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&");

/** Relativer Weg von einer Seite (z. B. "doku/anleitung/") zur Wurzel. */
const rootFor = (urlPath) => "../".repeat(urlPath.split("/").filter(Boolean).length) || "./";

/** Wie GitHub Überschriften-Anker bildet (github-slugger). */
function makeSlugger() {
  const seen = new Map();
  return (text) => {
    const base = text
      .toLowerCase()
      .trim()
      .replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, "")
      .replace(/ /g, "-");
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    return n === 0 ? base : `${base}-${n}`;
  };
}

async function readVersion() {
  const conf = JSON.parse(await readFile(path.join(ROOT, "src-tauri/tauri.conf.json"), "utf8"));
  const version = conf.version;
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`Version in src-tauri/tauri.conf.json ungültig: ${version}`);
  }
  return version;
}

/** Asset-Namen wie in scripts/release-notes.sh bzw. release.yml. */
function downloads(version) {
  const tag = `v${version}`;
  const files = {
    windows: `Code-Pruefer_${version}_x64-setup.exe`,
    macArm: `Code-Pruefer_${version}_aarch64.dmg`,
    macIntel: `Code-Pruefer_${version}_x64.dmg`,
    sums: "SHA256SUMS.txt",
  };
  const vars = {
    version,
    repo: REPO,
    release: `${REPO}/releases/tag/${tag}`,
    releases: `${REPO}/releases`,
  };
  for (const [k, f] of Object.entries(files)) {
    vars[`file.${k}`] = f;
    vars[`dl.${k}`] = `${REPO}/releases/download/${tag}/${f}`;
  }
  return vars;
}

function fillPlaceholders(html, vars, where) {
  return html.replace(/\{\{([\w.]+)\}\}/g, (_, key) => {
    if (!(key in vars)) throw new Error(`Unbekannter Platzhalter {{${key}}} in ${where}`);
    return escapeHtml(vars[key]);
  });
}

// ---------------------------------------------------------------------------
// Markdown -> HTML

/** Wandelt ein einfaches Mermaid-Flussdiagramm in eine nummerierte Liste. */
function flowchartToList(src) {
  const labels = new Map();
  const order = [];
  const optional = new Set();
  for (const line of src.split("\n")) {
    for (const m of line.matchAll(/(\w+)\["([^"]*)"\]/g)) {
      if (!labels.has(m[1])) {
        labels.set(m[1], m[2]);
        order.push(m[1]);
      }
    }
    const opt = line.match(/-\.\s*\w+\s*\.->\s*(\w+)/);
    if (opt) optional.add(opt[1]);
  }
  if (order.length === 0) return null;
  const items = order.map((id) => {
    const text = escapeHtml(labels.get(id).replace(/<br\s*\/?>/gi, " ").replace(/\s+/g, " ").trim());
    return optional.has(id)
      ? `<li class="flow-optional"><em>optional:</em> ${text}</li>`
      : `<li>${text}</li>`;
  });
  return `<ol class="flow">\n${items.join("\n")}\n</ol>\n`;
}

/** Repository-Pfad -> Doku-Unterseite (falls veröffentlicht). */
const DOC_BY_SRC = new Map(DOCS.map((d) => [d.src, d]));

// Erlaubte Link-Ziele im Markdown: https:, http:, mailto:, relative Pfade und
// #-Anker. Alles andere (javascript:, data:, vbscript:, file: ...) bricht den
// Build ab, damit aus einem Dokument im Repository kein Skript-Link wird.
const ALLOWED_SCHEMES = new Set(["https", "http", "mailto"]);

/** Kurzer, lesbarer Ausschnitt für Fehlermeldungen. */
const excerpt = (s) => JSON.stringify(String(s).length > 80 ? `${String(s).slice(0, 80)}…` : String(s));

/**
 * Prüft ein Link-Ziel aus dem Markdown und gibt es bereinigt zurück.
 * Browser entfernen führende Leer- und Steuerzeichen sowie Tab und
 * Zeilenumbruch mitten in der URL; "\x01JaVaScRiPt:" wäre also ein
 * Skript-Link. Deshalb wird erst so normalisiert wie im Browser und dann das
 * Schema (ohne Rücksicht auf Groß-/Kleinschreibung) geprüft.
 */
export function checkHref(href, where) {
  const raw = String(href ?? "");
  const url = raw.replace(/^[\u0000-\u0020]+|[\u0000-\u0020]+$/g, "").replace(/[\t\n\r]/g, "");
  const fail = (why) => {
    throw new Error(`Unsicherer Link in ${where}: ${why}: ${excerpt(raw)}`);
  };
  if (/[\u0000-\u001f\u007f]/.test(url)) fail("Steuerzeichen im Link");
  if (url === "" || url.startsWith("#")) return url;
  // Zeichenreferenzen wie "&#106;" oder "&colon;" vor dem Pfad würde der
  // Browser auflösen; sie könnten ein Schema verstecken.
  // Bis zum ersten "/" oder "?" gelesen, weil "#" zu "&#106;" gehören kann.
  const head = url.split(/[/?]/, 1)[0];
  if (/&(#|[a-z])/i.test(head)) fail("Zeichenreferenz im Schema-Teil");
  if (/^[/\\]{2}/.test(url)) fail("protokollrelative Links sind nicht erlaubt");
  if (url.startsWith("\\")) fail("Backslash am Anfang");
  const scheme = url.match(/^([a-z][a-z0-9+.-]*):/i);
  if (scheme) {
    if (!ALLOWED_SCHEMES.has(scheme[1].toLowerCase())) fail(`Schema "${scheme[1]}:" ist nicht erlaubt`);
  } else if (url.split(/[/?#]/, 1)[0].includes(":")) {
    // z. B. "1x:..." oder "-x:...": kein gültiges Schema, aber auch kein
    // eindeutiger relativer Pfad.
    fail("unklares Schema");
  }
  return url;
}

function rewriteHref(href, doc) {
  href = checkHref(href, doc.src);
  if (!href || href.startsWith("#") || /^[a-z][a-z0-9+.-]*:/i.test(href)) return href;
  const hashAt = href.indexOf("#");
  const rawPath = hashAt >= 0 ? href.slice(0, hashAt) : href;
  const hash = hashAt >= 0 ? href.slice(hashAt) : "";
  const target = path.posix.normalize(path.posix.join(path.posix.dirname(doc.src), decodeURI(rawPath)));
  if (target.startsWith("..")) throw new Error(`Link außerhalb des Repositorys in ${doc.src}: ${href}`);
  const mapped = DOC_BY_SRC.get(target.replace(/\/$/, ""));
  if (mapped) return `${rootFor(doc.slug)}${mapped.slug}${hash}`;
  const abs = path.join(ROOT, target);
  if (!existsSync(abs)) throw new Error(`Link auf eine fehlende Datei in ${doc.src}: ${href}`);
  const kind = statSync(abs).isDirectory() ? "tree" : "blob";
  const encoded = target.replace(/\/$/, "").split("/").map(encodeURIComponent).join("/");
  return `${REPO}/${kind}/main/${encoded}${hash}`;
}

// Rohes HTML im Markdown (Block und Inline, außerhalb von Code) ist nicht
// erlaubt: Die Doku-Seiten sollen nur das enthalten, was Markdown ausdrücken
// kann, und ein eingeschmuggeltes <script> oder <img onerror> darf nicht auf
// der Website landen. HTML-Kommentare (z. B. Hinweise an Bearbeiter) werden
// still entfernt. Eine Whitelist harmloser Tags gibt es bewusst nicht, weil
// die Quellen heute keine brauchen; wer z. B. <Version> schreiben will, nimmt
// einen Code-Span (`<Version>`).
const stripComments = (s) => s.replace(/<!--[\s\S]*?-->/g, "");

function checkTokens(tokens, marked, doc) {
  marked.walkTokens(tokens, (t) => {
    if (t.type === "html") {
      if (stripComments(t.raw).trim() !== "") {
        throw new Error(
          `Rohes HTML im Markdown ist nicht erlaubt (${doc.src}, ${t.block ? "Block" : "Inline"}): ${excerpt(t.raw.trim())}`,
        );
      }
    } else if (t.type === "link") {
      t.href = rewriteHref(t.href, doc);
    } else if (t.type === "image") {
      checkHref(t.href, doc.src);
      throw new Error(`Bilder im Markdown werden nicht unterstützt (${doc.src}): ${excerpt(t.raw)}`);
    }
  });
}

export function renderMarkdown(source, doc) {
  const md = source;
  const marked = new Marked({ gfm: true });
  const base = new Renderer();
  const tableLabel = doc.lang === "en" ? "Table" : "Tabelle";
  let tables = 0;
  marked.use({
    renderer: {
      heading(token) {
        const inner = this.parser.parseInline(token.tokens);
        return `<h${token.depth} id="${escapeHtml(token.id)}">${inner}</h${token.depth}>\n`;
      },
      code(token) {
        if (token.lang === "mermaid") {
          const list = flowchartToList(token.text);
          if (list) return list;
        }
        return `<pre tabindex="0"><code>${escapeHtml(token.text)}</code></pre>\n`;
      },
      html(token) {
        // checkTokens hat alles außer Kommentaren schon abgelehnt; escapen
        // bleibt als zweite Sicherung.
        const rest = stripComments(token.text);
        return rest.trim() ? escapeHtml(rest) : "";
      },
      table(token) {
        return `<div class="table-wrap" tabindex="0" role="region" aria-label="${tableLabel} ${++tables}">\n${base.table.call(this, token)}</div>\n`;
      },
    },
  });

  let tokens = marked.lexer(md);
  checkTokens(tokens, marked, doc);

  // Handgeschriebene Inhaltsverzeichnisse ("## Inhalt") entfallen; die Seite
  // bekommt ein erzeugtes Inhaltsverzeichnis.
  const out = [];
  let skipping = false;
  for (const t of tokens) {
    if (t.type === "heading" && t.depth <= 2) skipping = t.depth === 2 && t.text.trim() === "Inhalt";
    if (!skipping) out.push(t);
  }
  // Doppelte Trennlinien (vor und nach dem entfernten Abschnitt) zusammenfassen.
  tokens = out.filter((t, i, arr) => {
    if (t.type !== "hr") return true;
    for (let j = i - 1; j >= 0; j--) {
      if (arr[j].type === "space") continue;
      return arr[j].type !== "hr";
    }
    return true;
  });

  // Ohne Überschrift erster Ebene wird die erste Überschrift zur h1.
  const headings = tokens.filter((t) => t.type === "heading");
  if (!headings.some((h) => h.depth === 1) && headings.length > 0) headings[0].depth = 1;

  const slug = makeSlugger();
  const toc = [];
  for (const h of headings) {
    const plain = decodeEntities(marked.parseInline(h.text).replace(/<[^>]*>/g, ""));
    h.id = slug(plain);
    if (h.depth === 2) toc.push({ id: h.id, text: plain });
  }
  const title = headings.find((h) => h.depth === 1);

  const html = marked.parser(tokens);
  return {
    html,
    toc,
    h1: title ? decodeEntities(marked.parseInline(title.text).replace(/<[^>]*>/g, "")) : doc.title,
  };
}

// ---------------------------------------------------------------------------
// Seitenrahmen

function navLinks(items, root, current) {
  return items
    .map((it) => {
      const href = /^https?:/.test(it.href) ? it.href : `${root}${it.href}`;
      const attrs = [
        `href="${escapeHtml(href)}"`,
        it.key && it.key === current ? 'aria-current="page"' : "",
        it.lang ? `lang="${it.lang}" hreflang="${it.lang}"` : "",
        it.hreflang ? `hreflang="${it.hreflang}"` : "",
      ]
        .filter(Boolean)
        .join(" ");
      return `<li><a ${attrs}>${escapeHtml(it.label)}</a></li>`;
    })
    .join("\n          ");
}

function layout({ urlPath, title, description, lang = "de", nav, body, version, alternates = false, absoluteRoot = false, script = false }) {
  const root = absoluteRoot ? SITE_PATH : rootFor(urlPath);
  const ui = UI[lang] ?? UI.de;
  const canonical = new URL(urlPath, SITE_URL).href;
  const ogImage = new URL(OG_IMAGE, SITE_URL).href;
  const alt = alternates
    ? `\n  <link rel="alternate" hreflang="de" href="${SITE_URL}">\n  <link rel="alternate" hreflang="en" href="${SITE_URL}en/">`
    : "";
  return `<!doctype html>
<html lang="${lang}">
<head>
  <meta charset="utf-8">
  <meta http-equiv="Content-Security-Policy" content="${CSP}">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHtml(title)}</title>
  <meta name="description" content="${escapeHtml(description)}">
  <link rel="canonical" href="${canonical}">${alt}
  <meta name="color-scheme" content="light dark">
  <meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)">
  <meta name="theme-color" content="#0d1117" media="(prefers-color-scheme: dark)">
  <meta name="referrer" content="strict-origin-when-cross-origin">
  <link rel="icon" href="${root}img/favicon.png" type="image/png" sizes="32x32">
  <link rel="apple-touch-icon" href="${root}img/apple-touch-icon.png">
  <link rel="stylesheet" href="${root}assets/site.css">
  <meta property="og:type" content="website">
  <meta property="og:site_name" content="Code-Prüfer">
  <meta property="og:locale" content="${ui.locale}">
  <meta property="og:title" content="${escapeHtml(title)}">
  <meta property="og:description" content="${escapeHtml(description)}">
  <meta property="og:url" content="${canonical}">
  <meta property="og:image" content="${ogImage}">
  <meta property="og:image:type" content="image/png">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta property="og:image:alt" content="${escapeHtml(ui.ogAlt)}">
  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="${escapeHtml(title)}">
  <meta name="twitter:description" content="${escapeHtml(description)}">
  <meta name="twitter:image" content="${ogImage}">
  <meta name="twitter:image:alt" content="${escapeHtml(ui.ogAlt)}">${script ? `\n  <script src="${root}assets/site.js" defer></script>` : ""}
</head>
<body>
  <a class="skip-link" href="#inhalt">${ui.skip}</a>
  <header class="site-header">
    <div class="container">
      <a class="brand" href="${lang === "en" ? `${root}en/` : root}">
        <img src="${root}img/icon.png" alt="" width="32" height="32">
        <span>Code-Prüfer</span>
      </a>
      <nav class="site-nav" aria-label="${ui.mainNav}">
        <ul>
          ${navLinks(ui.nav, root, nav)}
        </ul>
      </nav>
    </div>
  </header>
  <main id="inhalt" tabindex="-1">
${body}
  </main>
  <footer class="site-footer">
    <div class="container">
      <ul>
        ${navLinks(ui.footer, root, null)}
      </ul>
      <p>${ui.footerLine(escapeHtml(version))}</p>
    </div>
  </footer>
</body>
</html>
`;
}

function docBody(doc, rendered, version) {
  const root = rootFor(doc.slug);
  const langAttr = doc.lang !== "de" ? ` lang="${doc.lang}"` : "";
  const toc =
    rendered.toc.length >= 3
      ? `<nav class="doc-toc" aria-label="Inhalt dieser Seite">
        <details data-toc>
          <summary>Inhalt dieser Seite</summary>
          <ol>
${rendered.toc.map((t) => `            <li><a href="#${escapeHtml(t.id)}">${escapeHtml(t.text)}</a></li>`).join("\n")}
          </ol>
        </details>
      </nav>`
      : "";
  return `    <div class="container">
      <div class="doc-layout${toc ? "" : " no-toc"}">
      ${toc}
      <article class="doc-content"${langAttr}>
        <nav class="breadcrumb" aria-label="Brotkrümelnavigation" lang="de">
          <ol>
            <li><a href="${root}">Start</a></li>
            <li><a href="${root}doku/">Doku</a></li>
            <li aria-current="page">${escapeHtml(doc.title)}</li>
          </ol>
        </nav>
${rendered.html}
        <p class="doc-meta" lang="de">Diese Seite wird automatisch aus
          <a href="${REPO}/blob/main/${doc.src.split("/").map(encodeURIComponent).join("/")}"><code>${escapeHtml(doc.src)}</code></a>
          im Repository erzeugt (Stand: Version ${escapeHtml(version)}).</p>
      </article>
      </div>
    </div>`;
}

function docsIndexBody() {
  const root = rootFor("doku/");
  const items = DOCS.map(
    (d) => `          <li>
            <a href="${root}${d.slug}">${escapeHtml(d.title)}</a>${d.lang === "en" ? ' <span lang="de">(englisch)</span>' : ""}
            <p>${escapeHtml(d.description)}</p>
          </li>`,
  ).join("\n");
  return `    <div class="container">
      <div class="page">
        <h1>Dokumentation</h1>
        <p class="lead">Alle Anleitungen zum Code-Prüfer. Die Seiten entstehen automatisch aus den Dokumenten im Repository und sind damit immer auf dem Stand des Quellcodes.</p>
        <ul class="doc-list">
${items}
        </ul>
        <p>Weitere Dokumente (Änderungsprotokoll, Sicherheitsrichtlinie, Lizenzen) liegen im
          <a href="${REPO}">Repository auf GitHub</a>, zum Beispiel das
          <a href="${REPO}/blob/main/CHANGELOG.md">Änderungsprotokoll</a> und die
          <a href="${REPO}/blob/main/SECURITY.md">Sicherheitsrichtlinie</a>.</p>
      </div>
    </div>`;
}

// ---------------------------------------------------------------------------

async function listHtml(dir, rel = "") {
  const out = [];
  for (const e of await readdir(dir, { withFileTypes: true })) {
    if (e.name.startsWith("_") || e.name.startsWith(".")) continue;
    const r = rel ? `${rel}/${e.name}` : e.name;
    if (e.isDirectory()) {
      if (r === "assets" || r === "img") continue;
      out.push(...(await listHtml(path.join(dir, e.name), r)));
    } else if (e.name.endsWith(".html")) out.push(r);
  }
  return out;
}

function parsePage(raw, file) {
  const m = raw.match(/^<!--\s*page:\s*(\{[\s\S]*?\})\s*-->\s*\n/);
  if (!m) throw new Error(`${file}: Kopfkommentar <!-- page: {...} --> fehlt`);
  return { meta: JSON.parse(m[1]), content: raw.slice(m[0].length) };
}

async function write(rel, content) {
  const file = path.join(OUT, rel);
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, content);
}

export async function build() {
  const version = await readVersion();
  const vars = downloads(version);
  await rm(OUT, { recursive: true, force: true });
  await mkdir(OUT, { recursive: true });

  await cp(path.join(SRC, "assets"), path.join(OUT, "assets"), { recursive: true });
  await cp(path.join(SRC, "img"), path.join(OUT, "img"), { recursive: true });
  // App-Icon direkt aus src-tauri/icons (nicht doppelt im Repository).
  await cp(path.join(ROOT, "src-tauri/icons/64x64.png"), path.join(OUT, "img/icon.png"));
  await cp(path.join(ROOT, "src-tauri/icons/32x32.png"), path.join(OUT, "img/favicon.png"));
  await cp(path.join(ROOT, "src-tauri/icons/128x128@2x.png"), path.join(OUT, "img/apple-touch-icon.png"));
  await writeFile(path.join(OUT, ".nojekyll"), "");

  const pages = [];

  for (const rel of await listHtml(SRC)) {
    const { meta, content } = parsePage(await readFile(path.join(SRC, rel), "utf8"), `site/${rel}`);
    const urlPath = rel === "index.html" ? "" : rel.endsWith("/index.html") ? rel.slice(0, -"index.html".length) : rel;
    const absoluteRoot = rel === "404.html";
    const root = absoluteRoot ? SITE_PATH : rootFor(urlPath);
    const body = fillPlaceholders(content, { ...vars, root }, `site/${rel}`);
    const title = meta.fullTitle ? meta.title : `${meta.title} – Code-Prüfer`;
    await write(
      rel,
      layout({
        urlPath,
        title,
        description: meta.description,
        lang: meta.lang ?? "de",
        nav: meta.nav,
        body,
        version,
        alternates: rel === "index.html" || rel === "en/index.html",
        absoluteRoot,
        script: body.includes("data-download"),
      }),
    );
    if (!absoluteRoot) pages.push(urlPath);
  }

  await write(
    "doku/index.html",
    layout({
      urlPath: "doku/",
      title: "Dokumentation – Code-Prüfer",
      description: "Anleitung, Hinweise zur Installation, Datenschutz, Code signing policy und Informationen zum Mitmachen.",
      nav: "doku",
      body: docsIndexBody(),
      version,
    }),
  );
  pages.push("doku/");

  for (const doc of DOCS) {
    const source = await readFile(path.join(ROOT, doc.src), "utf8");
    const rendered = renderMarkdown(source, doc);
    await write(
      `${doc.slug}index.html`,
      layout({
        urlPath: doc.slug,
        title: `${rendered.h1} – Code-Prüfer`,
        description: doc.description,
        nav: "doku",
        body: docBody(doc, rendered, version),
        version,
        script: true,
      }),
    );
    pages.push(doc.slug);
  }

  const urls = [...new Set(pages)].sort((a, b) => a.length - b.length || a.localeCompare(b));
  await write(
    "sitemap.xml",
    `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls.map((u) => `  <url><loc>${escapeHtml(new URL(u, SITE_URL).href)}</loc></url>`).join("\n")}
</urlset>
`,
  );
  await write("robots.txt", `User-agent: *\nAllow: /\n\nSitemap: ${SITE_URL}sitemap.xml\n`);

  for (const f of ["img/og-image.png"]) {
    if (!(await stat(path.join(OUT, f)).catch(() => null))) throw new Error(`${f} fehlt (scripts/site-screenshots.mjs)`);
  }
  console.log(`Website gebaut: ${path.relative(ROOT, OUT)}/ (${urls.length} Seiten, Version ${version})`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  build().catch((err) => {
    console.error(err instanceof Error ? err.message : err);
    process.exit(1);
  });
}

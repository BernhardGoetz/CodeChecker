// Tests für die Markdown-Umwandlung der Website (scripts/build-site.mjs):
// rohes HTML und unsichere Link-Schemata brechen den Build ab.
//
//   npm run test:site
import { test } from "node:test";
import assert from "node:assert/strict";
import { checkHref, renderMarkdown } from "./build-site.mjs";

const doc = { src: "README.md", slug: "doku/anleitung/", title: "Anleitung", lang: "de" };
const render = (md) => renderMarkdown(md, doc).html;

test("rohes HTML wird abgelehnt", () => {
  assert.throws(() => render("# T\n\n<script>alert(1)</script>\n"), /Rohes HTML.*README\.md.*Block.*<script>/);
  assert.throws(() => render("# T\n\nText <img src=x onerror=alert(1)> Text\n"), /Rohes HTML.*Inline.*onerror/);
  assert.throws(() => render("# T\n\n<details><summary>x</summary>y</details>\n"), /Rohes HTML/);
  assert.throws(() => render("# T\n\nZeile<br>Zeile\n"), /Rohes HTML/);
  assert.throws(() => render("# T\n\n| a |\n|---|\n| <kbd>x</kbd> |\n"), /Rohes HTML/);
  assert.throws(() => render("# <b>T</b>\n"), /Rohes HTML/);
});

test("HTML-Kommentare werden still entfernt", () => {
  const html = render("# T\n\n<!-- Hinweis\nüber zwei Zeilen -->\n\nText <!-- inline --> weiter\n");
  assert.doesNotMatch(html, /<!--|Hinweis|inline/);
  assert.match(html, /Text  weiter/);
  assert.throws(() => render("# T\n\n<!-- offen\n\nText\n"), /Rohes HTML/);
});

test("unsichere Link-Schemata werden abgelehnt", () => {
  for (const href of ["javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<b>x</b>", "vbscript:x", "file:///etc/passwd"]) {
    assert.throws(() => render(`# T\n\n[x](${href})\n`), /Unsicherer Link in README\.md/, href);
  }
  assert.throws(() => render("# T\n\n[x]( javascript:alert(1))\n"), /Unsicherer Link/);
  assert.throws(() => render("# T\n\n<javascript:alert(1)>\n"), /Unsicherer Link/);
  assert.throws(() => render("# T\n\n[x][r]\n\n[r]: javascript:alert(1)\n"), /Unsicherer Link/);
  assert.throws(() => render("# T\n\n![x](data:image/png;base64,AAAA)\n"), /Unsicherer Link/);
  assert.throws(() => render("# T\n\n[x](&#106;avascript:alert(1))\n"), /Unsicherer Link/);
});

test("checkHref normalisiert wie ein Browser", () => {
  for (const href of [" javascript:x", "\u0001javascript:x", "\tjava\nscript:x", "JAVASCRIPT:x", "java\u0000script:x", "//evil.example/", "\\\\evil"]) {
    assert.throws(() => checkHref(href, "t.md"), /Unsicherer Link in t\.md/, JSON.stringify(href));
  }
  assert.equal(checkHref(" https://example.org/ ", "t.md"), "https://example.org/");
  assert.equal(checkHref("mailto:a@example.org", "t.md"), "mailto:a@example.org");
  assert.equal(checkHref("#abschnitt", "t.md"), "#abschnitt");
});

test("normale Links und Code-Spans funktionieren", () => {
  const html = render(
    "# T\n\nSiehe [Doku](https://example.org/a?b=1), [Mail](mailto:a@example.org), " +
      "[Anker](#t), [Mitmachen](CONTRIBUTING.md#tests) und [Lizenz](LICENSE).\n\n" +
      "Datei `Code-Pruefer_<Version>_x64-setup.exe` und\n\n```\n<script>im Codeblock</script>\n```\n",
  );
  assert.match(html, /href="https:\/\/example\.org\/a\?b=1"/);
  assert.match(html, /href="mailto:a@example\.org"/);
  assert.match(html, /href="#t"/);
  assert.match(html, /href="\.\.\/\.\.\/doku\/mitmachen\/#tests"/);
  assert.match(html, /href="https:\/\/github\.com\/BernhardGoetz\/CodeChecker\/blob\/main\/LICENSE"/);
  assert.match(html, /<code>Code-Pruefer_&lt;Version&gt;_x64-setup\.exe<\/code>/);
  assert.match(html, /&lt;script&gt;im Codeblock&lt;\/script&gt;/);
  assert.doesNotMatch(html, /<script/);
});

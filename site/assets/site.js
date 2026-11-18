// Hebt den passenden Download-Knopf hervor (nur ein Hinweis; ohne
// JavaScript funktionieren alle Knöpfe genauso). Keine Daten werden
// gespeichert oder gesendet.
(function () {
  "use strict";
  var ua = navigator.userAgent || "";
  var platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  var key = null;
  if (/Win/i.test(platform) || /Windows NT/i.test(ua)) {
    key = "windows";
  } else if (/Mac/i.test(platform) || /Macintosh/i.test(ua)) {
    // Apple Silicon und Intel lassen sich im Browser nicht sicher
    // unterscheiden; ein WebGL-Renderer mit "Apple" deutet auf Apple Silicon.
    key = "mac-intel";
    try {
      var canvas = document.createElement("canvas");
      var gl = canvas.getContext("webgl");
      var ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
      var renderer = ext ? String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)) : "";
      if (/Apple/i.test(renderer) && !/Intel/i.test(renderer)) key = "mac-arm";
      else if (!renderer) key = null;
    } catch (e) {
      key = null;
    }
  }
  if (!key) return;
  var link = document.querySelector('[data-download="' + key + '"]');
  if (link) link.classList.add("is-suggested");
})();

// Inhaltsverzeichnis der Doku-Seiten: auf breiten Bildschirmen aufgeklappt.
(function () {
  "use strict";
  var toc = document.querySelector("details[data-toc]");
  if (toc && window.matchMedia && window.matchMedia("(min-width: 1000px)").matches) toc.open = true;
})();

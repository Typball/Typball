// Typst の文書の中の Mermaid を SVG にするページ (見えない WebView に読み込まれる。mermaid.rs)。
//
// - Rust → ページ: typballMermaid.render(id, code)
// - ページ → Rust: "ready" / "mermaid:<id>:ok:<SVG>" / "mermaid:<id>:err:<メッセージ>"
//
// 出す SVG は Typst (resvg) が描けるものにする: 文字は foreignObject ではなく <text> (htmlLabels を切る)、
// 大きさは viewBox から決めた数値、フォントは Typball が同梱している名前。
(function () {
  function post(message) {
    try {
      var host = window.typball_host;
      if (host && host.postMessage) host.postMessage(message);
    } catch (e) { /* 送れなくても続ける */ }
  }

  var FONT = '"Typball Sans", "Noto Sans JP", sans-serif';
  var seq = 0;
  // Mermaid は同時に 1 つしか描けないので、頼まれた順に 1 つずつ
  var chain = Promise.resolve();

  // Mermaid の SVG を Typst 向けに直す: 幅が "100%" なので viewBox の大きさを数値で入れ、画面用の style を外す
  function fix(svg) {
    var doc = new DOMParser().parseFromString(svg, "image/svg+xml");
    var root = doc.documentElement;
    if (!root || root.nodeName !== "svg" || doc.getElementsByTagName("parsererror").length > 0) {
      throw new Error("Mermaid produced an SVG that is not well-formed XML");
    }
    if (root.getElementsByTagName("foreignObject").length > 0) {
      throw new Error("this kind of Mermaid diagram draws its text as HTML, which Typst cannot render");
    }
    var box = (root.getAttribute("viewBox") || "").split(/[\s,]+/).map(Number);
    if (box.length === 4 && box[2] > 0 && box[3] > 0) {
      root.setAttribute("width", String(box[2]));
      root.setAttribute("height", String(box[3]));
    }
    root.removeAttribute("style");
    // 影のフィルターを外す: Typst はフィルターの掛かった図形を画像に変えてしまう (PDF でベクターにならない)
    var i;
    var filters = root.querySelectorAll("filter");
    for (i = 0; i < filters.length; i++) filters[i].remove();
    var all = root.querySelectorAll("*");
    for (i = 0; i < all.length; i++) {
      all[i].removeAttribute("filter");
      var inline = all[i].getAttribute("style");
      if (inline && /filter\s*:/.test(inline)) {
        all[i].setAttribute("style", inline.replace(/(^|;)\s*filter\s*:[^;]*/g, "$1"));
      }
    }
    var sheets = root.querySelectorAll("style");
    for (i = 0; i < sheets.length; i++) {
      sheets[i].textContent = sheets[i].textContent.replace(/([;{\s])filter\s*:[^;}]*;?/g, "$1");
    }
    return new XMLSerializer().serializeToString(root);
  }

  window.typballMermaid = {
    render: function (id, code) {
      chain = chain.then(function () {
        seq += 1;
        return window.mermaid.render("tbm" + seq, code);
      }).then(function (result) {
        post("mermaid:" + id + ":ok:" + fix(result.svg));
      }).catch(function (e) {
        // 書き間違いのとき、Mermaid はエラーの絵をページに残す
        var left = document.getElementById("dtbm" + seq);
        if (left) left.remove();
        post("mermaid:" + id + ":err:" + String((e && e.message) || e).split("\n")[0]);
      });
    }
  };

  document.addEventListener("DOMContentLoaded", function () {
    try {
      window.mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "default",
        htmlLabels: false,
        flowchart: { htmlLabels: false },
        fontFamily: FONT,
        themeVariables: { fontFamily: FONT }
      });
    } catch (e) { /* render が失敗を知らせる */ }
    var ready = function () { post("ready"); };
    // 図形の大きさは文字の幅で決まるので、フォントが読めてから始める (読めなくても始める)
    if (document.fonts && document.fonts.load) {
      Promise.all([
        document.fonts.load('16px "Typball Sans"'),
        document.fonts.load('bold 16px "Typball Sans"')
      ]).then(ready, ready);
    } else {
      ready();
    }
  });
})();

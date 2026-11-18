// サイトモードのスクロール同期。開発サーバーのページ (Hugo や Jekyll が作った HTML) に、Typball が
// 読み込みのたびに流し込む。
//
// サイトのページには「元の Markdown の何行目か」の手がかりが無い。そこで、Typball が同じ Markdown から
// 作った HTML (ブロックごとに data-line が付いている。Markdown モードの本文と同じもの) を受け取り、
// その各ブロックの文字 (と画像のファイル名) を、ページの見出し・段落・リストの項目・画像と上から順に
// 突き合わせて「行 ↔ ページの要素」の対応点を作る。対応点の間は行番号で比例配分する。
// テーマが足した部分 (ヘッダー・目次・サイドバー) やショートコードの出力は、合う相手が無いので飛ばされる。
//
// - typballSite.setSource(html, lines): Markdown から作った HTML と、文書の行数
// - typballSite.scrollToLine(n, anchor): エディタの行 n が画面の anchor の高さに来るようにスクロールする
// - 位置の計算と、スクロールを Rust へ知らせる処理は scrollsync.js (Markdown モードと共通)。
//   このファイルの前に流し込まれている
(function () {
  "use strict";
  if (window.typballSite) return;

  // 対応点がこの数より少なければ同期しない (別のページを見ている、ほとんど合わなかった)
  var MIN_ANCHORS = 3;
  // 突き合わせで、次の相手を探す範囲 (ページの要素の数)。最初の 1 つが見つかるまでは広く探す
  // (記事の前に、テーマのメニューなどの要素がたくさん並ぶ)
  var WINDOW_FIRST = 600;
  var WINDOW = 80;
  var CANDIDATES = "h1,h2,h3,h4,h5,h6,p,li,pre,blockquote,table,dt,dd,figcaption,img";

  var anchors = [];
  var totalLines = 1;

  function post(message) {
    try {
      var host = window.typball_host;
      if (host && host.postMessage) host.postMessage(message);
    } catch (e) { /* ホスト無し */ }
  }

  // 比べるための文字: 文字と数字だけを残す (空白・引用符・ダッシュの違い、見出しに付くリンクの記号を無視する)
  function textKey(text) {
    return (text || "").replace(/[^\p{L}\p{N}]+/gu, "").toLowerCase();
  }

  // 画像のファイル名 (クエリを外し、小文字に)
  function imageKey(src) {
    if (!src) return "";
    var name = src.split(/[?#]/)[0].split("/").pop() || "";
    try { name = decodeURIComponent(name); } catch (e) { /* そのまま */ }
    return name ? "img:" + name.toLowerCase() : "";
  }

  function keyOf(el) {
    if (el.tagName === "IMG") return imageKey(el.getAttribute("src"));
    var key = textKey(el.textContent);
    if (key) return key;
    // 文字の無いブロック (画像だけの段落) は、中の最初の画像で比べる
    var img = el.querySelector ? el.querySelector("img") : null;
    return img ? imageKey(img.getAttribute("src")) : "";
  }

  function sameKey(a, b) {
    if (a === b) return true;
    if (a.indexOf("img:") === 0 || b.indexOf("img:") === 0) return false;
    // 片方の後ろに何か付いている (テーマが足した記号、途中から違うショートコードの出力)
    var n = Math.min(a.length, b.length);
    return n >= 10 && (a.lastIndexOf(b, 0) === 0 || b.lastIndexOf(a, 0) === 0);
  }

  // Markdown から作った HTML → [{ line, key, heading }] (上から順)
  function sourceItems(html) {
    var doc = new DOMParser().parseFromString("<body>" + html + "</body>", "text/html");
    var marked = doc.querySelectorAll("[data-line]");
    var items = [];
    for (var i = 0; i < marked.length; i++) {
      var el = marked[i];
      var line = parseInt(el.getAttribute("data-line"), 10);
      if (!(line > 0)) continue;
      // リストの項目は、項目の中に置いた印に行番号が付いている
      var block = el.classList.contains("tb-mark") ? el.parentNode : el;
      var key = keyOf(block);
      if (!key) continue;
      var heading = !!(block.querySelector && block.querySelector("h1,h2,h3,h4,h5,h6")) &&
        block.children.length === 1;
      items.push({ line: line, key: key, heading: heading });
    }
    return items;
  }

  function visible(el) {
    return !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
  }

  function rebuild(html) {
    var items = sourceItems(html);
    var nodes = document.querySelectorAll(CANDIDATES);
    var cands = [];
    for (var i = 0; i < nodes.length; i++) {
      if (!visible(nodes[i])) continue;
      var key = keyOf(nodes[i]);
      if (key) cands.push({ el: nodes[i], key: key });
    }
    var found = [];
    var from = 0;
    for (var s = 0; s < items.length; s++) {
      var item = items[s];
      // 短い文字は、テーマの部品 (メニューの項目など) と偶然合いやすいので、見出し以外では使わない
      if (!item.heading && item.key.indexOf("img:") !== 0 && item.key.length < 4) continue;
      var end = Math.min(cands.length, from + (found.length ? WINDOW : WINDOW_FIRST));
      for (var k = from; k < end; k++) {
        // 見出しは見出しとだけ合わせる (テーマが目次を出していると、同じ文字のリストの項目が先にある)
        if (item.heading && !/^H[1-6]$/.test(cands[k].el.tagName)) continue;
        if (sameKey(item.key, cands[k].key)) {
          found.push({ line: item.line, el: cands[k].el });
          from = k + 1;
          break;
        }
      }
    }
    anchors = found;
  }

  // スクロールの同期 (scrollsync.js。Markdown モードと同じ計算)。対応点は、上の突き合わせで見つけたもの
  var sync = window.typballScrollSync({
    anchors: function () { return anchors; },
    totalLines: function () { return totalLines; },
    minAnchors: MIN_ANCHORS,
    post: post
  });

  window.typballSite = {
    setSource: function (html, lines) {
      totalLines = Math.max(1, lines || 1);
      try { rebuild(html || ""); } catch (e) { anchors = []; }
      sync.contentChanged();
    },
    scrollToLine: function (line, anchor) {
      sync.scrollToLine(line, anchor);
    },
    // 対応点の数 (動作の確認用)
    anchorCount: function () { return anchors.length; }
  };
})();

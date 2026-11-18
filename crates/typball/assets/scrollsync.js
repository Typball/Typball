// エディタとプレビューのスクロール同期の計算。Markdown モード (preview.js) とサイトモード (sitesync.js) が
// 同じものを使う。違うのは「行 ↔ ページの要素」の対応点の作り方だけ:
// - Markdown モード: Typball が作った HTML の data-line の付いた要素そのもの
// - サイトモード: 開発サーバーのページの要素を、文字と画像で突き合わせて見つけたもの
//
// 計算:
// - 対応点を (行, ページ上の y) の折れ線にする。先頭は 1 行目 = ページの上端、末尾は最後の行の次 = ページの
//   下端。行も y も前の点より先へ進む点だけを使うので、折れ線は必ず右上がりになる
// - エディタ → プレビュー: エディタの行 (小数部は、折り返された行の途中の位置) を折れ線で y に直し、その y が
//   画面の anchor (0 = 上端, 1 = 下端) の高さに来るようにスクロールする。エディタ側はスクロール位置
//   (先頭 0 〜 末尾 1) をそのまま anchor にして送るので、先頭では上端、末尾では下端で合い、途中は連続的に移る
// - プレビュー → エディタ: 画面の anchor の高さにある y を、同じ折れ線の逆で行に直して知らせる
//   ("line:<行>:<anchor>"。行は小数)
//
// 対応点の要素の高さは使わない。以前は「要素の上辺と下辺を anchor で混ぜた位置」から始めて次の対応点へ
// 比例配分していたが、背の高い要素 (画像、長い段落) の後ろで行き過ぎ、次の対応点で戻っていた
// (文書の後ろほど大きく、1 回で 100px 近く逆戻りした)。
(function () {
  "use strict";
  if (window.typballScrollSync) return;

  function clamp01(v) { return Math.max(0, Math.min(1, v)); }

  function topOf(el) {
    return el.getBoundingClientRect().top + window.scrollY;
  }

  function pageHeight() {
    return document.documentElement.scrollHeight;
  }

  // 行 (小数も可) → ページ上の y。折れ線の上を比例配分する
  function yOfLine(pts, line) {
    if (line <= pts[0].line) return pts[0].y;
    for (var i = 1; i < pts.length; i++) {
      if (line < pts[i].line) {
        var a = pts[i - 1], b = pts[i];
        return a.y + (b.y - a.y) * (line - a.line) / (b.line - a.line);
      }
    }
    return pts[pts.length - 1].y;
  }

  // ページ上の y → 行 (yOfLine の逆)
  function lineOfY(pts, y) {
    if (y <= pts[0].y) return pts[0].line;
    for (var i = 1; i < pts.length; i++) {
      if (y < pts[i].y) {
        var a = pts[i - 1], b = pts[i];
        return a.line + (b.line - a.line) * (y - a.y) / (b.y - a.y);
      }
    }
    return pts[pts.length - 1].line;
  }

  // 同期の処理を作る。opts:
  //   anchors():    今の対応点 [{ line, el }] (行の昇順)
  //   totalLines(): 文書の行数 (まだ分からなければ 0)
  //   minAnchors:   対応点がこれより少なければ同期しない (サイトモードで、別のページを見ているとき)
  //   post(text):   Rust へ知らせる
  window.typballScrollSync = function (opts) {
    // 最後にエディタから言われた行と、そのとき動かした位置 (向きを保つため)
    var last = null;
    // 自分で動かした位置 (その位置での scroll の知らせは、利用者の操作ではない)
    var expectedY = -1;
    var suppressUntil = 0;
    var pending = null;

    function debug(text) {
      // TYPBALL_SYNC_LOG を付けて起動したときだけ、計算の結果を Rust へ知らせる (webpreview.rs の sync_log)
      if (window.typballSyncDebug) opts.post("dbg:" + text);
    }

    function points() {
      var anchors = opts.anchors();
      var pts = [{ line: 1, y: 0 }];
      for (var i = 0; i < anchors.length; i++) {
        var tail = pts[pts.length - 1];
        var y = topOf(anchors[i].el);
        if (anchors[i].line <= tail.line || y < tail.y) continue;
        pts.push({ line: anchors[i].line, y: y });
      }
      var end = pts[pts.length - 1];
      var total = opts.totalLines();
      var endLine = total > 0 ? total + 1 : end.line + 1;
      var bottom = pageHeight();
      if (endLine > end.line && bottom >= end.y) pts.push({ line: endLine, y: bottom });
      return pts;
    }

    function enough() {
      return opts.anchors().length >= (opts.minAnchors || 1);
    }

    function scrollToLine(line, anchor) {
      if (!enough()) return;
      anchor = clamp01(anchor || 0);
      var y = yOfLine(points(), line);
      var vh = window.innerHeight;
      var max = Math.max(0, pageHeight() - vh);
      var target = Math.max(0, Math.min(max, y - anchor * vh - 12 * (1 - anchor)));
      // エディタが下へ進んだのにページが上へ戻る (その逆も) ことは無いようにする。行が進んでも y がほとんど
      // 進まない所 (空行が続く、ページに出ない部分) では、anchor が増えるぶんだけ少し戻ろうとする
      if (last) {
        if (line >= last.line && target < last.target) target = last.target;
        else if (line <= last.line && target > last.target) target = last.target;
      }
      last = { line: line, target: target };
      debug("scroll line=" + line + " anchor=" + anchor.toFixed(3) + " y=" + Math.round(y) +
        " target=" + Math.round(target) + " anchors=" + opts.anchors().length);
      suppressUntil = Date.now() + 500;
      expectedY = Math.round(target);
      // ページの CSS がなめらかなスクロールを指定していても、すぐその位置へ (途中の位置をエディタへ返さない)
      window.scrollTo({ top: target, left: window.scrollX, behavior: "instant" });
    }

    // 中身や対応点が変わった: 向きの記憶を捨て、少しの間、スクロールの知らせを止める
    // (中身の差し替えでページの位置がずれても、それをエディタへ返さない)
    function contentChanged() {
      last = null;
      suppressUntil = Date.now() + 500;
    }

    // ページのスクロール → エディタ
    window.addEventListener("scroll", function () {
      if (Date.now() < suppressUntil || pending) return;
      // 自分で動かした位置のままなら、利用者の操作ではない
      if (Math.abs(window.scrollY - expectedY) <= 1) return;
      pending = setTimeout(function () {
        pending = null;
        if (!enough()) return;
        // 利用者がページを動かした: エディタから言われた向きの記憶は捨てる
        last = null;
        expectedY = -1;
        var vh = window.innerHeight;
        var max = Math.max(0, pageHeight() - vh);
        var anchor = max > 0 ? clamp01(window.scrollY / max) : 0;
        var pts = points();
        var line = lineOfY(pts, window.scrollY + anchor * vh);
        // 最後の行の次 (ページの下端) までは行かせない
        line = Math.max(1, Math.min(pts[pts.length - 1].line - 0.01, line));
        debug("report y=" + Math.round(window.scrollY) + " line=" + line.toFixed(2) +
          " anchor=" + anchor.toFixed(3));
        opts.post("line:" + line.toFixed(2) + ":" + anchor.toFixed(3));
      }, 120);
    }, { passive: true });

    return { scrollToLine: scrollToLine, contentChanged: contentChanged };
  };
})();

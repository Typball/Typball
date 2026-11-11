# vendor/

## wxdragon-sys

crates.io の [wxdragon-sys](https://crates.io/crates/wxdragon-sys) 0.9.24 を丸ごとコピーし、6 ファイルだけ手を入れたものです。
ルートの `Cargo.toml` の `[patch.crates-io]` が crates.io 版の代わりにこれを使わせます。cargo はパス依存をこのフォルダから
そのままビルドする (C++ のラッパー `cpp/` も CMake でここから組む) ので、変更していないファイルも含めて全部が必要です。

wxWidgets 本体のソース (約 230 MB) と WebView2 SDK は、ビルド時に `build.rs` がダウンロードして `target/<profile>/wxWidgets`
に展開します。リポジトリには入りません (`.gitignore` の `/target/`)。

### Typball の変更 ("Typball patch" で検索)

| ファイル | 変更 |
|---|---|
| `build.rs` | 環境変数 `WXDRAGON_WEBVIEW_EDGE_STATIC` を CMake の `WXD_WEBVIEW_EDGE_STATIC` に渡す。WebView2 SDK を Rust 側の (TLS を検証する) ダウンローダーで先に取得する (`ensure_webview2_sdk`) |
| `cpp/CMakeLists.txt` | `WXD_WEBVIEW_EDGE_STATIC` があれば MinGW でも `wxUSE_WEBVIEW_EDGE_STATIC` をその値にする |
| `cpp/include/dialogs/wxd_dialogs.h`、`cpp/src/file_dialog.cpp` | `wxd_FileDialog_ShowModalWithChoices` を追加: ファイルダイアログにラベル付きの選択リストを足して表示する (保存ダイアログの文字コードと改行コードの選択に使う)。Windows は IFileDialog を直接組み立て、リストを「保存」ボタンの左に並べる (Windows がそこに置けるのは 1 組だけなので、残りは同じ行に自前の Static + ComboBox を作る。幅が足りないときは Windows の正規の段に切り替える)。ほかは wxFileDialogCustomizeHook |
| `cpp/include/core/wxd_window_base.h`、`cpp/src/window.cpp` | `wxd_Window_BindAfterPaint` を追加: ウィンドウが自分を描いた直後に、同じ描画の DC を渡してコールバックを呼ぶ (描画ハンドラが戻った後。ウィンドウは二重バッファにする)。エディタの上に不可視文字の記号を描き足すのに使う (wxStyledTextCtrl は描画ハンドラの終わりにバッファを写すので、その前に描いたものは消える) |

crates.io 版から外したのは、パッケージの管理用ファイル (`.cargo-ok`・`Cargo.lock`・`Cargo.toml.orig`) だけです。

### 版を上げるとき

1. 新しい版を crates.io から取得する (例: 一度 `[patch.crates-io]` を外してビルドし、`~/.cargo/registry/src/*/wxdragon-sys-<ver>/` を使う)
2. そのフォルダの中身でここを置き換え、上の表のファイルに "Typball patch" の変更を入れ直す
3. ルートの `Cargo.toml` の `wxdragon` の版と、`Cargo.lock` を合わせて更新する

変更前の版と見比べるには、crates.io 版との差分を取ります:

```bash
diff -r ~/.cargo/registry/src/index.crates.io-*/wxdragon-sys-0.9.24 vendor/wxdragon-sys
```

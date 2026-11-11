# vendor/

## wxdragon-sys

crates.io の [wxdragon-sys](https://crates.io/crates/wxdragon-sys) 0.9.24 を丸ごとコピーし、4 ファイルだけ手を入れたものです。
ルートの `Cargo.toml` の `[patch.crates-io]` が crates.io 版の代わりにこれを使わせます。cargo はパス依存をこのフォルダから
そのままビルドする (C++ のラッパー `cpp/` も CMake でここから組む) ので、変更していないファイルも含めて全部が必要です。

wxWidgets 本体のソース (約 230 MB) と WebView2 SDK は、ビルド時に `build.rs` がダウンロードして `target/<profile>/wxWidgets`
に展開します。リポジトリには入りません (`.gitignore` の `/target/`)。

### Typball の変更 ("Typball patch" で検索)

| ファイル | 変更 |
|---|---|
| `build.rs` | 環境変数 `WXDRAGON_WEBVIEW_EDGE_STATIC` を CMake の `WXD_WEBVIEW_EDGE_STATIC` に渡す。WebView2 SDK を Rust 側の (TLS を検証する) ダウンローダーで先に取得する (`ensure_webview2_sdk`) |
| `cpp/CMakeLists.txt` | `WXD_WEBVIEW_EDGE_STATIC` があれば MinGW でも `wxUSE_WEBVIEW_EDGE_STATIC` をその値にする |
| `cpp/include/dialogs/wxd_dialogs.h`、`cpp/src/file_dialog.cpp` | `wxd_FileDialog_ShowModalWithChoice` を追加: ファイルダイアログにラベル付きの選択リストを 1 つ足して表示する (wxFileDialogCustomizeHook。保存ダイアログの文字コード選択に使う) |

crates.io 版から外したのは、パッケージの管理用ファイル (`.cargo-ok`・`Cargo.lock`・`Cargo.toml.orig`) だけです。

### 版を上げるとき

1. 新しい版を crates.io から取得する (例: 一度 `[patch.crates-io]` を外してビルドし、`~/.cargo/registry/src/*/wxdragon-sys-<ver>/` を使う)
2. そのフォルダの中身でここを置き換え、上の表のファイルに "Typball patch" の変更を入れ直す
3. ルートの `Cargo.toml` の `wxdragon` の版と、`Cargo.lock` を合わせて更新する

変更前の版と見比べるには、crates.io 版との差分を取ります:

```bash
diff -r ~/.cargo/registry/src/index.crates.io-*/wxdragon-sys-0.9.24 vendor/wxdragon-sys
```

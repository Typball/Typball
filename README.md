# Typball

Typst のプレビュー付きタブエディタです。中央でソースを編集すると、右のプレビューが即座に更新され、
左のアウトラインには見出しが並びます。Markdown もプレビューでき (画像・表・コードの色付け)、
Hugo などのサイトのプロジェクト内なら開発サーバーの表示をそのまま確認できます。`.txt` も開けるので
普段使いのテキストエディタとしても使えます。

Windows / macOS / Linux で動くネイティブアプリで、Windows の日本語入力 (IME) をそのまま使えます。

## 主な機能

| | |
|---|---|
| Typst | [Typst](https://typst.app/) 0.15 を内蔵しているので、ネットワークや別途のインストールは不要です。日本語フォント Noto Sans JP も内蔵しているため、どの OS でも同じ見た目で組めます。PDF に書き出せます |
| Markdown | 表・脚注・タスクリスト・GFM アラート・コードの色付け・画像に対応したプレビュー。エディタとプレビューのスクロールが連動します |
| サイトモード | Hugo などのプロジェクト内の `.md` を開くと開発サーバーを起動し、実際のサイトの見た目でプレビューします。保存すると更新されます |
| エディタ | 日本語対応の等幅フォント UDEV Gothic を内蔵 (半角と全角が 1:2 で桁がそろう)。Typst と Markdown の構文の色分け (Markdown は ```` ```rust ```` のように言語を書いたコードブロックの中も)、行番号 (左・右・両側)、ルーラー、折り返し (なし / ウィンドウの右端 / 指定文字数)、対応括弧の強調、編集した行の強調 (未保存はオレンジ、保存済みは青)、不可視文字の記号表示 (表示 > 記号を表示: 改行は CRLF・LF・CR で形の違う矢印、ファイルの終わり [EOF]、空白・タブ・全角空白。打ち込んだ文字と紛れないように図形で描きます。新規文書の改行コードと違う改行は種類ごとに色を変えて強調) |
| 検索・置換 | 入力に合わせて探すインクリメンタル検索。大文字小文字の区別・単語単位・正規表現、すべて置換 |
| アクション | 太字・斜体・見出し・コード・リンク・箇条書きなどを、エディタ上部のボタンやメニューから付け外しできます |
| タブとウィンドウ | 1 つのウィンドウに複数のタブ。タブをウィンドウの外へドラッグすると新しいウィンドウになり、別のウィンドウへも移せます |
| ペイン | アウトライン・プレビュー・コンソールを、エディタの左・右・下のどこにでも置けます。折りたたみや別ウィンドウへの切り離しもできます |
| プレビューウィンドウ | エディタ無しで、プレビューだけを表示するウィンドウでも開けます |
| ファイル | `.typ` / `.md` / `.txt`。ウィンドウへのドラッグ&ドロップで開けます。ほかのアプリで書き換えられたファイルは、確認してから読み直します。Shift_JIS / EUC-JP / UTF-16 などの文字コードも判定して、同じ文字コード・同じ改行コードのまま保存します (保存時の文字コードと改行コードは「名前を付けて保存」のダイアログで選べます。読み間違えたときは ファイル > 文字コードを指定して開き直す)。改行コード (CRLF / LF / CR) は 編集 > 改行コード でも変換でき、今の改行コードはステータスバーの右端に出ます (新規文書の改行コードと違うときは太字、改行コードが混ざっているときは「混在」の警告)。新規文書の改行コードは 設定 > ファイル で選べます (初期値は OS の標準) |
| 壊れにくい保存 | 一時ファイルに書き終えてから入れ替えるので、保存中に USB メモリが外れても元のファイルは残ります (保存先に一時ファイルを作りたくなければ、設定で直接書き込みにも切り替えられます)。応答の無いドライブで固まらず、未保存の文書は数秒ごとに控えを取って、落ちても次の起動で復元します |
| 自動更新 | 新しい版が出ると知らせ、Windows ではダウンロードから再起動まで自動で行います |
| 表示言語 | 日本語 / 英語 (既定は OS の言語に合わせます) |

## ダウンロード

[Releases](https://github.com/Typball/Typball/releases) から OS に合ったファイルを取得してください。

| OS | ファイル | 使い方 |
|---|---|---|
| Windows | `typball-<ver>-windows-x86_64-setup.exe` | インストーラー。管理者権限は不要です。`.typ` / `.md` の関連付けとスタートメニューへの登録ができます (関連付けは後から 設定 → 関連付け でも変えられます) |
| Windows | `typball-<ver>-windows-x86_64.zip` | 展開した `Typball.exe` をそのまま実行します (インストール不要) |
| macOS | `typball-<ver>-macos-aarch64.tar.gz` (Apple Silicon) / `-macos-x86_64.tar.gz` (Intel) | 展開した `Typball.app` を「アプリケーション」へ。署名していないため、初回は右クリック →「開く」で起動してください |
| Linux | `typball-<ver>-linux-x86_64.tar.gz` | 展開した `Typball` を実行します (GTK3 が必要です) |

Windows の Markdown プレビューには Microsoft Edge WebView2 ランタイムを使います。Windows 11 には標準で入っています。
入っていない場合は、インストーラーが導入を案内します。

## 使い方

### 基本

- 新しいファイルは「ファイル → 新規」(Ctrl+N) で作ります。Typst は A4・Noto Sans JP・日本語の設定と見出し 1 つの雛形から始まります
  (雛形は 設定 → ファイル で変えられます)。
- ファイルはメニューから開くほか、ウィンドウのどこへドラッグ&ドロップしても開けます。
- プレビュー・アウトライン・コンソールは「表示」メニューで出し入れします。ペインのヘッダの ▾ メニューやドラッグで、
  左・右・下へ置き場を変えられます。
- コンソールには Typst のエラーや警告が出ます。`行:桁` の付いた行をダブルクリックすると、その行へ移動します。
- Typst は「アクション → PDF に書き出し」(Ctrl+E) で PDF にできます。

### キーボードショートカット

| キー | 操作 |
|---|---|
| Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+S | 新規 / 開く / 上書き保存 / 名前を付けて保存 |
| Ctrl+W | タブを閉じる |
| Ctrl+Shift+N | 新しいウィンドウ |
| Ctrl+F / Ctrl+H | 検索 / 置換 |
| F3 / Shift+F3 | 次を検索 / 前を検索 |
| Ctrl+G | 指定行へ移動 |
| Ctrl+B / Ctrl+I | 太字 / 斜体 (Typst・Markdown) |
| Ctrl+E | PDF に書き出し (Typst) |
| Ctrl+1 / Ctrl+2 / Ctrl+3 | 折り返さない / 指定文字数で折り返し / ウィンドウの右端で折り返し |
| Ctrl+Shift+L / Ctrl+Shift+P / Ctrl+Shift+K | アウトライン / プレビュー / コンソールの表示切り替え |

macOS では Ctrl の代わりに ⌘ を使います。

### コマンドライン

```bash
Typball ファイル...                  # 開く (設定に従ってエディタかプレビューウィンドウで)
Typball --edit ファイル              # エディタで開く
Typball --view ファイル              # プレビューウィンドウで開く
Typball --new-instance ファイル      # 起動中の Typball に渡さず、別のプロセスで開く
```

Typball が起動しているときにファイルをダブルクリックすると、起動中の Typball のタブに入ります。
インストーラー版では、ファイルの右クリックメニューに「Typball で編集」「Typball でプレビュー」が加わります。

### 関連付け

「ツール → 設定 → 関連付け」で、`.txt` / `.md` / `.typ` をダブルクリックしたときに Typball で開くよう OS に登録できます
(インストーラーを使わない zip 版でも、インストーラーと同じ登録ができます)。形式ごとに今の既定のアプリと Typball の登録状況が出て、
「関連付ける」「解除」はその場で反映されます。登録されるのは、そのとき動いている Typball です
(別の場所に置いた Typball が登録されているときは、その場所が表示され、「関連付ける」で付け替えられます)。

| OS | |
|---|---|
| Windows | ファイルに付くアイコンを、書類のアイコン / Typball のアイコン / 好きなファイル (`.ico` など) から選べます。Windows の設定で別のアプリを既定に選んである拡張子は、登録しただけでは既定のアプリが変わりません。「既定のアプリの設定を開く」から Typball を選んでください |
| macOS | その種類のファイルを開く既定のアプリを Typball にします (Finder の「このアプリケーションで開く → すべてを変更」と同じ)。`Typball.app` として起動しているときだけ使えます。解除するとテキストエディットに戻ります |
| Linux | `~/.local/share/applications/typball.desktop` と `~/.config/mimeapps.list` に登録します。ファイルのアイコンはデスクトップのアイコンテーマで決まります |

### 設定

設定は「ツール → 設定」で変更します。内容は `Settings.toml` という 1 つのファイルに保存されます。

| 場所 | |
|---|---|
| 実行ファイルと同じフォルダ | `Settings.toml` を置くと、そちらが優先されます (USB メモリでの持ち運びや、PC 間の移行に) |
| Windows | `%APPDATA%\Typball\Settings.toml` |
| macOS | `~/Library/Application Support/Typball/Settings.toml` |
| Linux | `~/.config/Typball/Settings.toml` |

使っているファイルの場所は、設定ダイアログの下部に表示されます。ファイルに無い項目は既定値で補われるので、
版をまたいでそのままコピーできます。直接編集する場合の項目は次のとおりです。

```toml
[general]
language = "system"   # "system" / "ja" / "en"
last_tab = "close_window"   # 最後のタブを閉じたとき: "keep_empty" (空のタブを残す) / "close_window" (ウィンドウを閉じる。最後のウィンドウなら終了)
show_toolbar = true         # メニューの下にツールバーを表示する (表示メニューの「ツールバー」と同じ)

[editor]
font_face = "UDEV Gothic"   # 既定は同梱の等幅フォント UDEV Gothic (未インストールでも使える)。エディタの中身だけに効く (ルーラー・行番号・コンソール・タブなどの UI は OS の既定 GUI フォント)
font_size = 12
line_spacing = 1            # 行間: 1 行の高さに足す px (0 でフォントの行の高さのまま)
tab_width = 4               # タブ幅 (半角の文字数)
use_tabs = false            # Tab キーでタブ文字を入力する (false は次のタブ位置までのスペース)
backspace = "soft_tab"      # Backspace でスペースを消すとき: "char" (1 個ずつ) / "soft_tab" (タブ位置で、その前がタブ幅ぶんスペースのときだけまとめて) / "tab_stop" (タブ位置にあるとき、続いているスペースをタブ幅ぶんまで) / "full_tab_stop" (前のタブ位置までスペースだけのとき、そこまで) / "to_tab_stop" (前のタブ位置まで、続いているスペースを) / "indent" (行頭の字下げだけ、前のタブ位置まで)
wrap = "window"             # 折り返し: "none" / "window" (ウィンドウの右端) / "column" (指定文字数)
wrap_column = 80            # wrap = "column" のときの文字数
line_numbers = "left"       # "none" / "left" / "right" / "both"
highlight_changes = true    # 編集した行を強調する (未保存はオレンジの帯 + 太字の行番号、保存済みは青の帯)
saved_changes = "keep"      # 保存したとき: "keep" (色を変えて残す) / "clear" (強調を消す)
show_ruler = true           # エディタ上部にルーラーを表示する
ruler_tabs = "arrow"        # ルーラーでのタブ位置の示し方: "none" (目盛りだけ) / "arrow" (小さな矢印)
show_eol = true             # 改行を矢印で表示する (CRLF は折れた矢印、LF は ↓、CR は ←。新規文書の改行コードと違うものは色を変える)
show_eof = true             # ファイルの終わりに [EOF] を表示する
show_whitespace = true      # 空白・タブ・全角空白を記号で表示する

[files]
default_filetype = "txt"    # 新規タブ (Ctrl+N) の形式: "txt" / "md" / "typ"
external_change = "ask"     # 外部で変更されたとき: "ask" (確認) / "reload" (自動で読み直す) / "ignore"
untitled_numbering = true   # 新規文書の名前に番号を付ける ("無題-1.txt"。false なら "無題.txt")
default_encoding = "utf-8"  # 新規文書の文字コード: "utf-8" / "utf-8-bom" / "utf-16le" / "utf-16be" / "shift_jis" / "euc-jp" / "iso-2022-jp" など
default_line_ending = "auto" # 新規文書の改行コード: "auto" (OS の標準) / "crlf" / "lf" / "cr"
fallback_encoding = "auto"  # UTF-8 として読めないファイルの文字コード ("auto" は内容から推定)
large_file_mb = 16          # これを超える大きさのファイルは確認してから、色分け・編集行の強調・アウトライン・プレビューを止めて開く (0 で無効)
max_file_mb = 256           # これを超える大きさのファイルは開かない (0 で無制限)
save_method = "atomic"      # 保存のしかた: "atomic" (一時ファイルに書いてから入れ替える) / "direct" (元のファイルに直接書き込み、保存先に一時ファイルを作らない)
recovery_backup = true      # 未保存の文書を数秒ごとに控えて、落ちたときに次の起動で復元する

[filetypes.typ]             # 形式ごとに、開いたときに表示するペインとバー (書かない項目は形式の既定値)
show_outline = true
show_preview = true
show_console = true         # コンソール (Typst の診断 / Markdown の変換結果 / 開発サーバーの出力)
show_actionbar = true
sync_scroll_to_preview = true   # エディタをスクロールしたらプレビューも追従 (Markdown のプレビューのみ対応)
sync_scroll_to_editor = true    # プレビューをスクロールしたらエディタも追従 (エディタを動かしたくなければ false)
open_mode = "editor"        # ファイルを渡されて起動したとき (ダブルクリック): "editor" / "viewer" (プレビューウィンドウ)。--edit / --view が優先
template = """           # 新しいファイル (Ctrl+N / 起動時の新規タブ) の雛形。無ければ言語ごとの既定 (下の内容)
#set page(paper: "a4")
#set text(font: "Noto Sans JP", lang: "ja", size: 11pt)

= 新しい文書

"""
[filetypes.typ.pane_sizes]  # ペインの大きさ (px)。分割線を動かすと形式ごとに保存される (左右の置き場は幅、下は高さ)
left = 200
right = 460
bottom = 110
[filetypes.md]
show_outline = true
show_preview = true
show_actionbar = true
[filetypes.txt]
show_outline = false

[panes]                     # ペインの置き場: "left" / "right" / "bottom" (ヘッダの ▾ メニューかドラッグで変わる)
outline = "left"
preview = "right"
console = "bottom"
bottom_width = "editor"     # 下の置き場の幅: "editor" (エディタの幅) / "window" (ウィンドウの幅いっぱい)

[preview]                   # プレビュー
font_face = "Noto Sans JP"  # プレビューの既定フォント: Typst は文書が #set text(font:) で指定しないときの既定、Markdown の本文、プレビューウィンドウ。同梱の Noto Sans JP は未インストールでも使える
font_size = 12              # 同じくサイズ (pt)
markdown_mode = "auto"      # "auto" (サイトのプロジェクト内ならサイトモード) / "markdown" / "site"
site_command = "hugo server -D -N -M --port {port} -s {root}"   # 開発サーバーのコマンド ({port} は空きポート、{root} はプロジェクトのフォルダ)
site_url = "http://localhost:{port}/"                            # 表示する URL

[instance]                  # Typball をもう一度起動したとき
mode = "single"             # "single" (起動中の Typball にファイルを渡す。--new-instance で一時的に別プロセス) / "multiple" (起動のたびに別プロセス)

[update]                    # 更新の確認 (GitHub Releases)
check_on_startup = true     # 起動時に新しい版が公開されていないか確認する
channel = "stable"          # "stable" (番号付きの Release だけ) / "nightly" (push ごとの開発版も含める)
interval_hours = 24         # 確認の間隔 (時間)
last_check = 0              # 最後に確認した時刻 (UNIX 秒、自動で更新される)
skipped_version = ""        # 「この版はスキップ」を選んだバージョン

[window]              # 終了時に自動保存され、次回起動時に復元される (初回は画面の縦横半分の大きさで中央に出る)
width = 1200          # 幅と高さは拡大率 100% 換算
height = 800
x = 100               # 位置は画面の座標。タイトルバーがどの画面からも掴めない位置のときだけ、画面の中へ戻す
y = 100
```

### 更新

起動時 (既定で 24 時間に 1 回) と「ヘルプ → 更新を確認」で、新しい版が公開されていないか確かめます。
Windows では「更新して再起動」を選ぶと、ダウンロード・入れ替え・再起動まで自動で行い、開いていたファイルを開き直します。
macOS / Linux では新しい版を知らせ、ダウンロードページを開きます。設定 → 更新 で、開発版 (nightly) も受け取るようにできます。

## ビルド方法

### Windows (ワンクリック)

リポジトリ直下の [`build.cmd`](build.cmd) をダブルクリックするか、ターミナルで実行すると `dist\Typball.exe` ができます。
足りない開発ツールは winget で自動的にインストールします。初回は wxWidgets をソースからビルドするため 10 分以上かかります。

```bat
build.cmd
```

| 必要なもの | 導入方法 (無い場合は build.cmd が実行) | 備考 |
|---|---|---|
| Rust | `winget install Rustlang.Rustup` | その後 `stable-x86_64-pc-windows-gnu` ツールチェーンを追加 |
| C/C++ コンパイラ・CMake・make・libclang | `winget install CodeBlocks.CodeBlocks.MinGW --scope machine` | Code::Blocks 25.03 同梱の MinGW-w64 (GCC 14, UCRT) |
| CMake (予備) | `winget install Kitware.CMake` | 見つかった MinGW に cmake.exe が無い場合のみ |
| libclang (予備) | `winget install LLVM.LLVM` | 見つかった MinGW に libclang.dll が無い場合のみ |

| 環境変数 | 値 | 意味 |
|---|---|---|
| `TYPBALL_TOOLCHAIN` | `gnu` (既定) / `msvc` | `msvc` にすると Visual Studio 2022 Build Tools (C++) を使い、無ければ winget で導入します |
| `TYPBALL_MINGW_DIR` | パス | `gcc.exe` / `g++.exe` のあるフォルダを指定する |
| `TYPBALL_PROFILE` | `debug` | debug ビルドにする (既定は release) |
| `TYPBALL_DRY_RUN` | `1` | インストールとビルドを実行せず、実行するコマンドを表示するだけにする |

### 手動でビルドする

```bash
cargo build --release
```

- **Windows (MinGW)**: `rustup toolchain install stable-x86_64-pc-windows-gnu` の上で、gcc / g++ / cmake / mingw32-make / libclang
  (Code::Blocks 同梱の MinGW で可) を PATH に通し、`RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu` を付けてビルドします。
- **Windows (MSVC)**: Visual Studio Build Tools (C++)・CMake・LLVM (libclang) を入れれば、既定の MSVC ツールチェーンでもビルドできます。
- **macOS**: Xcode Command Line Tools と CMake (`brew install cmake`)。
- **Linux (Debian / Ubuntu)**:
  ```bash
  sudo apt-get install build-essential cmake pkg-config libclang-dev \
    libgtk-3-dev libpng-dev libjpeg-dev libtiff-dev libexpat1-dev zlib1g-dev \
    libgl1-mesa-dev libglu1-mesa-dev libxkbcommon-dev libwayland-dev \
    libxtst-dev libsm-dev libice-dev libnotify-dev \
    libwebkit2gtk-4.1-dev libsoup-3.0-dev
  ```

初回のビルドでは wxWidgets のソース (約 230 MB。Windows は WebView2 SDK も) をダウンロードするため、ネットワーク接続が必要です。
実行ファイルは `target/release/Typball` (Windows は `Typball.exe`) にできます。

## ライセンス

Typball 本体は [MIT License](LICENSE) です。

実行ファイルには次のフォントとソフトウェアが含まれており、それぞれのライセンスに従います。
ライセンスの全文は [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) にまとめてあり、配布物 (zip・インストーラー・
macOS / Linux のアーカイブ) にも同梱しています。アプリでは ヘルプ → バージョン情報 →「License」で一覧を確認できます。

| 含まれるもの | ライセンス |
|---|---|
| Noto Sans JP (Regular / Bold)。© 2014-2021 Adobe | SIL Open Font License 1.1 |
| UDEV Gothic (Regular / Bold)。© 2022 Yuko OTAWARA。BIZ UDゴシック (© 2022 The BIZ UDGothic Project Authors) と JetBrains Mono (© 2020 The JetBrains Mono Project Authors) の合成 | SIL Open Font License 1.1 |
| Typst 同梱フォント: Libertinus Serif | SIL Open Font License 1.1 |
| Typst 同梱フォント: New Computer Modern / NewCM Math | GUST Font License 1.0 (NewCM10-Regular は GPL-3.0 以降 + Font Exception + Distribution Exception) |
| Typst 同梱フォント: DejaVu Sans Mono | Bitstream Vera / Arev Fonts License (DejaVu の変更部分はパブリックドメイン) |
| Typst 同梱の PDF 標準フォント (Foxit) | BSD-3-Clause |
| Typst 同梱のデータ (ICC プロファイル、HTML / MathML のデータ) | CC0 1.0 / CC BY 4.0 / BSD-3-Clause / W3C Document License |
| [Typst](https://github.com/typst/typst) | Apache License 2.0 |
| [wxWidgets](https://www.wxwidgets.org/) | wxWindows Library Licence 3.1 |
| wxWidgets に内蔵: Scintilla / Lexilla | Scintilla License |
| wxWidgets に内蔵: libpng | PNG Reference Library License v2 |
| wxWidgets に内蔵: zlib・NanoSVG | zlib License |
| wxWidgets に内蔵: libjpeg (Independent JPEG Group) | IJG License |
| wxWidgets に内蔵: libtiff | libtiff License |
| wxWidgets に内蔵: Expat | MIT |
| wxWidgets に内蔵: PCRE2 | BSD-3-Clause (PCRE2 の例外付き) |
| Microsoft Edge WebView2 SDK | BSD-3-Clause |
| Rust クレート (約 380 個。一覧と全文は THIRD-PARTY-NOTICES.txt) | MIT・Apache-2.0・Unicode-3.0・BSD・ISC・Zlib・MPL-2.0 など。選べるものは MIT (MIT の全文が無いものは Apache-2.0) で利用 |

This software is based in part on the work of the Independent JPEG Group.

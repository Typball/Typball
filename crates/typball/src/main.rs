//! Typball: Typst のプレビュー付きタブエディタ (wxWidgets / wxDragon)
//!
//! - `window.rs`          メインウィンドウ (メニュー・タブ管理・ファイル操作・外部変更の監視)
//! - `viewer.rs`          プレビュー専用ウィンドウ (`--view`、ファイル → プレビューウィンドウで開く)
//! - `appicon.rs`         ウィンドウのアイコン (Windows は exe に埋め込んだアイコン) とバージョン情報の大きな絵
//! - `tabbar.rs`          自前描画のタブバー (アイコン・閉じるボタン・ドラッグ)
//! - `tab.rs`             1 タブ分のページ (アウトライン | ツールバー + ルーラー + エディタ + 行番号 | プレビュー)
//! - `pane.rs`            サイドペインのヘッダ (切り離し・閉じる) と浮動ウィンドウ
//! - `actions.rs`         形式ごとの操作 (アクション) の一覧。メニューとアクションバーの元
//! - `actionbar.rs`       アクションバー (アイコンの並び) と書式付けの編集処理
//! - `editmenu.rs`        「編集」メニューとエディタの右クリックメニュー (フォーカス先への振り分け)
//! - `findbar.rs`         検索 / 置換バー (「検索」メニューから出す)
//! - `theme.rs`           自前描画の部品で共通の配色・フォント・描画ヘルパ
//! - `document.rs`        ファイル 1 つ分のメタ情報 (パス・種別・更新時刻)
//! - `filetype.rs`        ファイル種別 (.txt / .md / .typ) ごとの性質
//! - `outline.rs`         見出し一覧の抽出
//! - `editor.rs`          エディタ (Scintilla) の見た目・シンタックスハイライト・編集行の追跡
//! - `gutter.rs`          自前描画のルーラーと右側行番号
//! - `scintilla.rs`       wxDragon が公開していない Scintilla メッセージの送信
//! - `preview.rs`         Typst をコンパイルしてプレビューを描く
//! - `webpreview.rs`      WebView のプレビュー (Markdown → HTML、サイトモード)
//! - `mdhtml.rs`          Markdown → HTML の変換 (画像 URL の書き換え・コードの色付け)
//! - `site.rs`            静的サイトジェネレーターのプロジェクト検出と開発サーバーの起動
//! - `webview2_loader.rs` WebView2 の自前ローダー (MinGW ビルドで WebView2Loader.dll を不要にする)
//! - `update.rs`          GitHub Releases での更新の確認・取得・検証・自分自身の入れ替え
//! - `update_dialog.rs`   更新のダイアログと進捗表示 (ヘルプ → 更新を確認、起動時の自動確認)
//! - `instance.rs`        シングルインスタンス (2 つ目の起動は先行プロセスにファイルを渡して終わる)
//! - `fonts_win.rs`       同梱フォント (Noto Sans JP) を Windows の GDI にプロセス専用で登録する
//! - `settings.rs`        設定の構造体と TOML ファイル (Settings.toml) の読み書き
//! - `settings_dialog.rs` 設定ダイアログ
//! - `i18n.rs`            UI 文字列の多言語対応
//! - `ime_win.rs`         Windows の IME 変換ウィンドウ位置の補正
//! - `dnd_win.rs`         Windows のファイルのドロップ先 (Scintilla のテキストドロップと両立させる)
#![windows_subsystem = "windows"]

mod actionbar;
mod actions;
mod appicon;
#[cfg(windows)]
mod dnd_win;
mod document;
mod editmenu;
mod editor;
mod filetype;
mod findbar;
#[cfg(windows)]
mod fonts_win;
mod gutter;
mod i18n;
#[cfg(windows)]
mod ime_win;
mod instance;
mod mdhtml;
mod outline;
mod pane;
mod preview;
mod scintilla;
mod settings;
mod settings_dialog;
mod site;
mod tab;
mod tabbar;
mod theme;
mod update;
mod update_dialog;
mod viewer;
mod webpreview;
#[cfg(all(windows, target_env = "gnu"))]
mod webview2_loader;
mod window;
mod winplace;

use std::path::PathBuf;

use filetype::FileType;
use i18n::tr_args;
use instance::{Claim, Request};
use settings::{InstanceMode, OpenMode, Settings};
use window::WindowRef;

/// コマンドライン: 開き方 (`--edit` = エディタ、`--view` = プレビュー専用ウィンドウ。どちらも無ければ
/// 形式ごとの設定 `[filetypes.*] open_mode`)、`--new-instance` (先行プロセスに渡さず別プロセスで起動)、
/// 開くファイル (絶対パスにして返す)
struct Args {
    mode: Option<OpenMode>,
    new_instance: bool,
    files: Vec<PathBuf>,
}

fn parse_args() -> Args {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut args = Args {
        mode: None,
        new_instance: false,
        files: Vec::new(),
    };
    for arg in std::env::args_os().skip(1) {
        if arg == "--view" || arg == "-v" {
            args.mode = Some(OpenMode::Viewer);
            continue;
        }
        if arg == "--edit" || arg == "-e" {
            args.mode = Some(OpenMode::Editor);
            continue;
        }
        if arg == "--new-instance" {
            args.new_instance = true;
            continue;
        }
        let p = PathBuf::from(arg);
        if p.as_os_str().is_empty() {
            continue;
        }
        args.files
            .push(if p.is_absolute() { p } else { cwd.join(p) });
    }
    args
}

fn main() {
    let (loaded, load_error) = Settings::load();
    let args = parse_args();

    // シングルインスタンス: 先に動いている Typball があればファイルを渡して、このプロセスは終わる
    // (GUI を作る前に済ませるので、ダブルクリックしたファイルはすぐ既存のウィンドウに出る)
    let server = if loaded.instance.mode == InstanceMode::Single && !args.new_instance {
        let request = Request {
            mode: args.mode,
            files: args.files.clone(),
        };
        match instance::claim(&request) {
            Claim::Forwarded => return,
            Claim::Owner(server) => Some(server),
            Claim::Alone => None,
        }
    } else {
        None
    };

    // 同梱の Noto Sans JP を wx の部品 (プレビューウィンドウのテキスト表示・設定ダイアログ) でも
    // 使えるようにこのプロセスに登録する
    #[cfg(windows)]
    fonts_win::register_embedded();
    // システムフォントの検索は数百 ms かかるので、最初のプレビューを待たせないよう先に別スレッドで済ませる
    std::thread::spawn(typball_core::warm_up);
    // コードの色付け用の構文定義の読み込みも同様
    std::thread::spawn(mdhtml::warm_up);
    let _ = wxdragon::main(move |_| {
        i18n::set_language(&loaded.general.language);
        settings::set(loaded.clone());
        // Typst の既定フォント (文書が #set text(font:) で指定しないときに使う)
        typball_core::set_default_text(&loaded.preview.font_face, loaded.preview.font_size as f64);

        let win = open_at_startup(args.mode, args.files, &loaded);
        if let (Some(win), Some(e)) = (&win, load_error) {
            win.borrow()
                .frame
                .set_status_text(&tr_args("status.settings_load_error", &[("error", &e)]), 0);
        }
        // 前回の更新で残ったダウンロードを片付け、間隔が空いていれば新しい版を確認する (別スレッド)
        std::thread::spawn(update::cleanup_downloads);
        if let Some(win) = &win {
            update_dialog::check_on_startup(win);
        }
        // 後から起動した Typball からの要求を受ける。受信スレッドから GUI スレッドへ渡す
        if let Some(server) = server {
            server.listen(|request| {
                wxdragon::call_after(Box::new(move || open_request(request)));
                // call_after の中身はアイドル時に実行されるので、何も起きていないアプリを起こす
                wxdragon::wake_up_idle();
            });
        }
    });
    // サイトモードで起動した開発サーバー (子プロセス) は親が終わっても残るので、ここで止める
    site::stop_all();
}

/// ファイルを、プレビューウィンドウで開くものとエディタで開くものに分ける
/// (`--edit` / `--view` が無ければ形式ごとの設定 `open_mode` に従う)
fn split_by_mode(
    mode: Option<OpenMode>,
    files: Vec<PathBuf>,
    settings: &Settings,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    files.into_iter().partition(|p| {
        mode.unwrap_or_else(|| FileType::from_path(p).open_mode(settings)) == OpenMode::Viewer
    })
}

/// 起動時にファイルを開く。エディタのウィンドウを作ったらそれを返す (プレビューウィンドウだけなら None)
fn open_at_startup(
    mode: Option<OpenMode>,
    files: Vec<PathBuf>,
    settings: &Settings,
) -> Option<WindowRef> {
    let (view, edit) = split_by_mode(mode, files, settings);
    if edit.is_empty() && !view.is_empty() {
        // プレビューだけ: 1 つでも開けたらエディタは出さない。1 つも開けなければ空のエディタで起動する
        if viewer::open(&view) > 0 {
            return None;
        }
        return Some(window::create(&[], None));
    }
    viewer::open(&view);
    // ファイルが無ければ既定の形式の新規タブ (形式ごとの雛形) を 1 つ開く
    Some(window::create(&edit, None))
}

/// 後から起動した Typball から渡された要求を開く (GUI スレッド)。エディタで開くファイルは最後に
/// アクティブだったウィンドウのタブに入れ、そのウィンドウを前面に出す。ファイル無しの要求
/// (Typball をもう一度起動しただけ) はウィンドウを前面に出すだけ
fn open_request(request: Request) {
    let settings = settings::get();
    let (view, edit) = split_by_mode(request.mode, request.files, &settings);
    for path in &view {
        if let Some(v) = viewer::open_one(path) {
            window::bring_to_front(&v.frame);
        }
    }
    if edit.is_empty() && !view.is_empty() {
        return;
    }
    let win = match window::active_window() {
        Some(win) => {
            window::open_files(&win, &edit);
            win
        }
        None => window::create(&edit, None),
    };
    let frame = win.borrow().frame;
    window::bring_to_front(&frame);
}

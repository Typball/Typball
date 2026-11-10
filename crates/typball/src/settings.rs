//! アプリ設定
//!
//! 設定は TOML ファイル 1 つに保存する ([`path`] を参照)。
//! 構造体に `#[serde(default)]` を付けてあるので、古いファイルに無い項目は既定値で埋まり、
//! 知らない項目は無視される。したがって項目を追加・削除してもファイルの移行作業は不要。
//!
//! 設定項目を増やす手順:
//! 1. 該当セクションの構造体にフィールドを足し、`Default` に既定値を書く
//! 2. `settings_dialog.rs` の該当ページにウィジェットと読み取りクロージャを足す
//! 3. 値を使う側 (`main.rs` の `apply_settings` など) で参照する

use std::cell::RefCell;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 設定ファイル名。実行ファイルと同じフォルダにこの名前のファイルがあればそれを優先する
/// (ポータブル運用・設定移行用)。
pub const FILE_NAME: &str = "Settings.toml";

/// 設定ディレクトリ名 (OS の設定フォルダ配下)
const APP_DIR: &str = "Typball";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub general: General,
    pub editor: Editor,
    pub files: Files,
    pub filetypes: FileTypes,
    pub panes: PaneLayout,
    pub preview: Preview,
    pub update: Update,
    pub window: Window,
    pub instance: Instance,
    pub export: Export,
}

/// 配色 (ウィンドウ・エディタ・プレビューで別々に選ぶ)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// OS の設定 (アプリのモード) に従う
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Self::System, Self::Light, Self::Dark];
}

/// Typball をもう一度起動したとき (関連付けのダブルクリックなど) にどうするか
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceMode {
    /// 先に動いている Typball にファイルを渡して、そちらで開く (`instance.rs`)
    #[default]
    Single,
    /// 起動のたびに別のプロセスになる (タブの移動は同じプロセスのウィンドウ間だけ)
    Multiple,
}

impl InstanceMode {
    pub const ALL: [InstanceMode; 2] = [Self::Single, Self::Multiple];
}

/// 起動のしかた
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Instance {
    pub mode: InstanceMode,
}

/// 更新チャネル
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// タグを打って公開した Release だけ (既定)
    #[default]
    Stable,
    /// 既定ブランチへの push ごとに CI が組んだビルド (固定タグ `nightly` の pre-release) も対象にする
    Nightly,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Self::Stable, Self::Nightly];
}

/// 更新の確認 (GitHub Releases)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Update {
    /// 起動時に新しい版があるか確認する
    pub check_on_startup: bool,
    /// 安定版だけか、開発版 (nightly) も受け取るか
    pub channel: Channel,
    /// 確認の間隔 (時間)。前回の確認からこれだけ経っていなければ起動時に確認しない
    pub interval_hours: u32,
    /// 最後に確認した時刻 (UNIX 秒)
    pub last_check: u64,
    /// 「この版はスキップ」を選んだバージョン (その版は起動時に知らせない)
    pub skipped_version: String,
}

impl Default for Update {
    fn default() -> Self {
        Self {
            check_on_startup: true,
            channel: Channel::Stable,
            interval_hours: 24,
            last_check: 0,
            skipped_version: String::new(),
        }
    }
}

/// Markdown のプレビューの方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkdownMode {
    /// サイト (Hugo など) のプロジェクトの中ならサイトモード、そうでなければ Markdown モード
    #[default]
    Auto,
    /// 常に Markdown を HTML にして表示する (入力に追従)
    Markdown,
    /// 常に開発サーバーの出力を表示する (保存で更新)
    Site,
}

impl MarkdownMode {
    pub const ALL: [MarkdownMode; 3] = [Self::Auto, Self::Markdown, Self::Site];
}

/// プレビューペイン (Markdown / サイトモード) の設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    /// プレビューの既定フォント: Typst は文書が `#set text(font:)` で指定しないときの既定、Markdown は本文、
    /// プレビューウィンドウのテキスト表示にも使う。既定は同梱の Noto Sans JP
    pub font_face: String,
    /// 同じくサイズ (pt)
    pub font_size: i32,
    pub markdown_mode: MarkdownMode,
    /// 開発サーバーの起動コマンド。`{port}` と `{root}` (プロジェクトのフォルダ) が置き換わる
    pub site_command: String,
    /// WebView に表示する URL。`{port}` が置き換わる
    pub site_url: String,
    /// プレビューの配色。Typst は暗いとき明るさを反転して描き、Markdown は CSS を切り替える
    /// (サイトモードはサイト自身の CSS に任せる)
    pub theme: Theme,
    /// Typst のプレビューの倍率: "fit_width" (幅に合わせる) / "fit_page" (ページ全体) / "125" (%)。
    /// プレビューのツールバーで最後に選んだもの
    pub typst_zoom: String,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            font_face: typball_core::EMBEDDED_FONT_FAMILY.to_string(),
            font_size: 11,
            markdown_mode: MarkdownMode::Auto,
            // -D: 下書きも出す、-N: 保存したページへ移動、-M: 出力をディスクに書かない
            site_command: "hugo server -D -N -M --port {port} -s {root}".to_string(),
            site_url: "http://localhost:{port}/".to_string(),
            theme: Theme::System,
            typst_zoom: "fit_width".to_string(),
        }
    }
}

/// 書き出し (アクション → 書き出し) で最後に選んだ形式と設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Export {
    /// Typst の形式: "pdf" / "png" (1 ファイル) / "png_pages" (ページごと) / "svg" / "svg_pages"
    pub typst_format: String,
    /// Markdown の形式: "pdf" / "html"
    pub markdown_format: String,
    /// PDF のバージョン ("1.7" など) と PDF/A ("" = 無し、"a-2b" など)
    pub pdf_version: String,
    pub pdf_a: String,
    pub pdf_ua: bool,
    pub pdf_tagged: bool,
    /// PNG の解像度 (dpi)
    pub png_dpi: u32,
    /// HTML に画像を埋め込む (data: URL)
    pub html_embed_images: bool,
    /// Markdown の PDF の用紙 ("A4" など) と向き
    pub paper: String,
    pub landscape: bool,
}

impl Default for Export {
    fn default() -> Self {
        Self {
            typst_format: "pdf".to_string(),
            markdown_format: "pdf".to_string(),
            pdf_version: "1.7".to_string(),
            pdf_a: String::new(),
            pdf_ua: false,
            pdf_tagged: true,
            png_dpi: 144,
            html_embed_images: true,
            paper: "A4".to_string(),
            landscape: false,
        }
    }
}

/// サイドペインを置く場所
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanePosition {
    Left,
    Right,
    Bottom,
}

impl PanePosition {
    pub const ALL: [PanePosition; 3] = [Self::Left, Self::Right, Self::Bottom];

    /// 左右の縦長の置き場か (下は横長)
    pub fn is_side(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    pub fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Bottom => 2,
        }
    }
}

/// 下の置き場の幅
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BottomWidth {
    /// エディタの幅 (左右のペインはウィンドウの下端まで伸びる)
    #[default]
    Editor,
    /// ウィンドウの幅 (左右のペインの下にも広がる)
    Window,
}

impl BottomWidth {
    pub const ALL: [BottomWidth; 2] = [Self::Editor, Self::Window];
}

/// ペインの配置 (全ウィンドウ・全タブで共通。ペインのヘッダのメニューから変える)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneLayout {
    pub outline: PanePosition,
    pub preview: PanePosition,
    /// コンソール (Typst のコンパイル結果・Markdown の変換結果・開発サーバーの出力)
    pub console: PanePosition,
    /// 下の置き場をエディタの幅にするか、ウィンドウの幅いっぱいにするか
    pub bottom_width: BottomWidth,
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self {
            outline: PanePosition::Left,
            preview: PanePosition::Right,
            console: PanePosition::Bottom,
            bottom_width: BottomWidth::Editor,
        }
    }
}

/// 一般
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// 表示言語。"system" で OS に従う。それ以外は locales/ の言語コード ("ja", "en")
    pub language: String,
    /// ウィンドウの最後のタブを閉じたときの動作
    pub last_tab: LastTab,
    /// ウィンドウ (メニュー・タブ・ペイン・ダイアログ) の配色
    pub theme: Theme,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: crate::i18n::SYSTEM.to_string(),
            last_tab: LastTab::default(),
            theme: Theme::System,
        }
    }
}

/// 最後のタブを閉じたとき
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LastTab {
    /// 空のタブを 1 つ開いたままにする
    #[default]
    KeepEmpty,
    /// ウィンドウを閉じる (最後のウィンドウなら Typball が終了する)
    CloseWindow,
}

impl LastTab {
    pub const ALL: [LastTab; 2] = [Self::KeepEmpty, Self::CloseWindow];
}

/// 行番号を表示する位置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineNumbers {
    None,
    #[default]
    Left,
    Right,
    Both,
}

impl LineNumbers {
    /// 設定ダイアログの選択肢の並び順
    pub const ALL: [LineNumbers; 4] = [Self::None, Self::Left, Self::Right, Self::Both];

    pub fn left(self) -> bool {
        matches!(self, Self::Left | Self::Both)
    }

    pub fn right(self) -> bool {
        matches!(self, Self::Right | Self::Both)
    }
}

/// 行の折り返し
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wrap {
    /// 折り返さない (横スクロール)
    None,
    /// ウィンドウの右端で折り返す
    #[default]
    Window,
    /// 指定した文字数 (`wrap_column`) で折り返す
    Column,
}

impl Wrap {
    pub const ALL: [Wrap; 3] = [Self::None, Self::Window, Self::Column];
}

/// 保存したとき、編集した行の強調をどうするか
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedChanges {
    /// 太字をやめ、色を変えて残す (保存前の編集行と区別できる)
    #[default]
    Keep,
    /// 強調を消す
    Clear,
}

/// エディタ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Editor {
    pub font_face: String,
    pub font_size: i32,
    pub tab_width: i32,
    /// 行の折り返し ("none" / "window" / "column")
    pub wrap: Wrap,
    /// `wrap = "column"` のときの 1 行の文字数 (半角の数字の幅で数える)
    pub wrap_column: i32,
    /// 行番号の位置 ("none" / "left" / "right" / "both")
    pub line_numbers: LineNumbers,
    /// 開いてから編集した行を強調する (行番号を太字にし、端に帯を出す)。
    /// ダイアログでは `saved_changes` と合わせて 1 つの選択肢 (変更行の強調) にしている
    pub highlight_changes: bool,
    /// 保存したときの編集行の強調 ("keep" = 色を変えて残す / "clear" = 消す)。
    /// 強調しないときも値は残す (メニューで強調を入れ直したときに戻る)
    pub saved_changes: SavedChanges,
    /// エディタ上部に文字位置のルーラーを表示する
    pub show_ruler: bool,
    /// エディタ (本文・行番号・ルーラー) の配色
    pub theme: Theme,
}

/// 既定のエディタフォント: 同梱の UDEV Gothic (日本語対応の等幅で、半角と全角が 1:2。どの OS でも同じ)。
/// 同梱の Noto Sans JP はプレビュー側の既定 (`Preview::font_face`)
pub const DEFAULT_FONT_FACE: &str = crate::fonts::EDITOR_FONT_FAMILY;

impl Default for Editor {
    fn default() -> Self {
        Self {
            font_face: DEFAULT_FONT_FACE.to_string(),
            font_size: 11,
            tab_width: 2,
            wrap: Wrap::Window,
            wrap_column: 80,
            line_numbers: LineNumbers::Left,
            highlight_changes: true,
            saved_changes: SavedChanges::Keep,
            show_ruler: true,
            theme: Theme::System,
        }
    }
}

/// ファイルの開き方・保存のしかた
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Files {
    /// 新規タブ (Ctrl+N) で作る形式の拡張子 ("txt" / "md" / "typ")
    pub default_filetype: String,
    /// 開いているファイルが外部で変更されたときの動作
    pub external_change: ExternalChange,
    /// 新規文書の名前に連番を付ける ("無題-1.txt")。false なら "無題.txt"
    pub untitled_numbering: bool,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            default_filetype: "txt".to_string(),
            external_change: ExternalChange::Ask,
            untitled_numbering: true,
        }
    }
}

/// 開いているファイルが外部で変更されたときの動作
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalChange {
    /// 確認ダイアログを出す
    #[default]
    Ask,
    /// 黙って読み直す (未保存の編集があるときだけ確認する)
    Reload,
    /// 読み直さない (ステータスバーに知らせるだけ)
    Ignore,
}

impl ExternalChange {
    pub const ALL: [ExternalChange; 3] = [Self::Ask, Self::Reload, Self::Ignore];
}

/// 種別ごとに、タブを開いたときにどのペイン・バーを出すか (解決済みの値)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Panes {
    pub show_outline: bool,
    pub show_preview: bool,
    /// コンソール (コンパイラ / 変換の出力)
    pub show_console: bool,
    /// アクションバー (太字・見出し・PDF 書き出しなどのアイコン)
    pub show_actionbar: bool,
    /// エディタをスクロールしたらプレビューも追従させる (今は Markdown のプレビューだけが対応)
    pub sync_scroll_to_preview: bool,
    /// プレビューをスクロールしたらエディタも追従させる (エディタを動かしたくないときは切る)
    pub sync_scroll_to_editor: bool,
}

/// ファイルを渡されて起動したとき (関連付けのダブルクリック、`Typball.exe ファイル`) の開き方。
/// コマンドラインの `--edit` / `--view` はこの設定より優先する
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenMode {
    /// エディタのタブで開く
    #[default]
    Editor,
    /// エディタ無しのプレビューウィンドウ (`viewer.rs`) で開く
    Viewer,
}

/// ペインの置き場の大きさ (px)。左右の置き場は幅、下は高さ。分割線を動かすと形式ごとに保存される
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneSizes {
    pub left: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Default for PaneSizes {
    fn default() -> Self {
        Self {
            left: 200,
            right: 460,
            bottom: 260,
        }
    }
}

impl PaneSizes {
    /// `PanePosition::index` の順 (左 / 右 / 下)
    pub fn as_array(self) -> [i32; 3] {
        [self.left, self.right, self.bottom]
    }

    pub fn from_array(sizes: [i32; 3]) -> Self {
        Self {
            left: sizes[0],
            right: sizes[1],
            bottom: sizes[2],
        }
    }
}

/// 設定ファイル上の種別ごとの設定 (`[filetypes.typ]` など)。ペインの項目は書かれていなければ (None)
/// 種別ごとの既定値 (`FileType::default_panes`) になるので、一部だけ書いても他の既定が消えない
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanesOverride {
    pub show_outline: Option<bool>,
    pub show_preview: Option<bool>,
    pub show_console: Option<bool>,
    pub show_actionbar: Option<bool>,
    pub sync_scroll_to_preview: Option<bool>,
    pub sync_scroll_to_editor: Option<bool>,
    /// ファイルを渡されて起動したときの開き方 (エディタ / プレビューウィンドウ)
    pub open_mode: OpenMode,
    /// ペインの置き場の大きさ (分割線を動かすと保存される。ダイアログには出ない)
    pub pane_sizes: PaneSizes,
    /// 新しいファイル (Ctrl+N、起動時の新規タブ) の雛形。None なら言語ごとの既定 (`FileType::default_template`)
    pub template: Option<String>,
}

impl PanesOverride {
    pub fn resolve(self, defaults: Panes) -> Panes {
        Panes {
            show_outline: self.show_outline.unwrap_or(defaults.show_outline),
            show_preview: self.show_preview.unwrap_or(defaults.show_preview),
            show_console: self.show_console.unwrap_or(defaults.show_console),
            show_actionbar: self.show_actionbar.unwrap_or(defaults.show_actionbar),
            sync_scroll_to_preview: self
                .sync_scroll_to_preview
                .unwrap_or(defaults.sync_scroll_to_preview),
            sync_scroll_to_editor: self
                .sync_scroll_to_editor
                .unwrap_or(defaults.sync_scroll_to_editor),
        }
    }

    /// ダイアログで決めたペインの表示をすべて明示的に保存する。ダイアログに無い項目 (開き方・大きさ) は
    /// `base` (今の値) のまま
    pub fn explicit(panes: Panes, base: PanesOverride) -> Self {
        Self {
            show_outline: Some(panes.show_outline),
            show_preview: Some(panes.show_preview),
            show_console: Some(panes.show_console),
            show_actionbar: Some(panes.show_actionbar),
            sync_scroll_to_preview: Some(panes.sync_scroll_to_preview),
            sync_scroll_to_editor: Some(panes.sync_scroll_to_editor),
            ..base
        }
    }
}

/// ファイル種別ごとの設定 (拡張子名のテーブル: [filetypes.typ] など)
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FileTypes {
    pub txt: PanesOverride,
    pub md: PanesOverride,
    pub typ: PanesOverride,
}

/// メインウィンドウの位置とサイズ (終了時に保存し、次回起動時に復元する。winplace.rs)
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    /// 幅と高さは DIP (拡大率 100% 換算)。復元時に置き先のモニターの拡大率で換算する。
    /// 未保存 (初回起動) なら None: 置き先のモニターの作業領域の縦横半分にする
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// 画面の座標 (実ピクセル)。未保存なら None (画面中央に出す)
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
}

/// 設定ファイルのパス。
/// 1. 実行ファイルと同じフォルダの `Settings.toml` があればそれ (ポータブル)
/// 2. なければ OS の設定フォルダ (`%APPDATA%\Typball`, `~/.config/Typball`,
///    `~/Library/Application Support/Typball`) の `Settings.toml`
pub fn path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let portable = dir.join(FILE_NAME);
            if portable.is_file() {
                return portable;
            }
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
        .join(FILE_NAME)
}

impl Settings {
    /// ファイルから読み込む。ファイルが無ければ既定値。
    /// 壊れていて読めない場合は既定値と一緒にエラーメッセージを返す (上書き保存で復旧できる)。
    pub fn load() -> (Settings, Option<String>) {
        let path = path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(s) => (s, None),
                Err(e) => (Settings::default(), Some(e.to_string())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => (Settings::default(), Some(e.to_string())),
        }
    }

    /// ファイルに保存する (ディレクトリが無ければ作る)
    pub fn save(&self) -> Result<(), String> {
        let path = path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let body = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let text = format!(
            "# Typball settings (https://github.com/Typball/Typball)\n\
             # 無い項目は既定値になります。このファイルをコピーすれば設定を移行できます。\n\n{body}"
        );
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }
}

thread_local! {
    static CURRENT: RefCell<Settings> = RefCell::new(Settings::default());
}

/// 現在有効な設定のコピーを返す
pub fn get() -> Settings {
    CURRENT.with(|c| c.borrow().clone())
}

/// 現在有効な設定を置き換える (保存はしない)
pub fn set(s: Settings) {
    CURRENT.with(|c| *c.borrow_mut() = s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_unknown_keys_are_tolerated() {
        let s: Settings = toml::from_str("[editor]\nfont_size = 14\n[future]\nx = 1\n").unwrap();
        assert_eq!(s.editor.font_size, 14);
        assert_eq!(s.editor.tab_width, 2);
        assert_eq!(s.general.language, "system");
        assert_eq!(s.editor.line_numbers, LineNumbers::Left);
    }

    #[test]
    fn filetype_tables_and_files_section() {
        let s: Settings = toml::from_str(
            "[files]\nexternal_change = \"reload\"\n[filetypes.typ]\nshow_preview = false\n",
        )
        .unwrap();
        assert_eq!(s.files.external_change, ExternalChange::Reload);
        assert_eq!(s.files.default_filetype, "txt");
        assert!(s.files.untitled_numbering);
        assert_eq!(s.filetypes.typ.show_preview, Some(false));
        // 書かれていない項目は種別の既定値のまま
        assert_eq!(s.filetypes.typ.show_outline, None);
        let typ = crate::filetype::FileType::Typ.panes(&s);
        assert!(typ.show_outline && !typ.show_preview && typ.show_actionbar);
        assert!(typ.sync_scroll_to_preview && typ.sync_scroll_to_editor);
        let s2: Settings =
            toml::from_str("[filetypes.md]\nsync_scroll_to_editor = false\n").unwrap();
        let md = crate::filetype::FileType::Md.panes(&s2);
        assert!(md.sync_scroll_to_preview && !md.sync_scroll_to_editor);
        // None はファイルに書き出さない
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("show_preview = false"), "{text}");
        assert!(!text.contains("show_outline"), "{text}");
    }

    #[test]
    fn line_numbers_are_lowercase_strings() {
        let s: Settings = toml::from_str(
            "[editor]
line_numbers = \"both\"
",
        )
        .unwrap();
        assert_eq!(s.editor.line_numbers, LineNumbers::Both);
        assert!(s.editor.line_numbers.left() && s.editor.line_numbers.right());
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("line_numbers = \"both\""), "{text}");
    }

    #[test]
    fn wrap_and_saved_changes_are_lowercase_strings() {
        let s: Settings =
            toml::from_str("[editor]\nwrap = \"column\"\nsaved_changes = \"clear\"\n").unwrap();
        assert_eq!(s.editor.wrap, Wrap::Column);
        assert_eq!(s.editor.wrap_column, 80);
        assert_eq!(s.editor.saved_changes, SavedChanges::Clear);
        let text = toml::to_string(&Settings::default()).unwrap();
        assert!(text.contains("wrap = \"window\""), "{text}");
        assert!(text.contains("saved_changes = \"keep\""), "{text}");
    }

    #[test]
    fn pane_positions_are_lowercase_strings() {
        let s: Settings = toml::from_str("[panes]\npreview = \"bottom\"\n").unwrap();
        assert_eq!(s.panes.preview, PanePosition::Bottom);
        assert_eq!(s.panes.outline, PanePosition::Left);
        assert!(toml::to_string(&s)
            .unwrap()
            .contains("preview = \"bottom\""));
    }

    #[test]
    fn bottom_width_and_open_mode() {
        let s: Settings = toml::from_str(
            "[panes]\nbottom_width = \"window\"\n[filetypes.typ]\nopen_mode = \"viewer\"\n",
        )
        .unwrap();
        assert_eq!(s.panes.bottom_width, BottomWidth::Window);
        assert_eq!(s.panes.console, PanePosition::Bottom);
        let typ = crate::filetype::FileType::Typ;
        assert_eq!(typ.open_mode(&s), OpenMode::Viewer);
        assert_eq!(
            crate::filetype::FileType::Md.open_mode(&s),
            OpenMode::Editor
        );
        // open_mode だけ書いてもペインの既定は残る
        assert!(typ.panes(&s).show_preview);
        let text = toml::to_string(&Settings::default()).unwrap();
        assert!(text.contains("bottom_width = \"editor\""), "{text}");
        assert!(text.contains("open_mode = \"editor\""), "{text}");
    }

    #[test]
    fn pane_sizes_are_per_filetype() {
        let s: Settings = toml::from_str(
            "[filetypes.md.pane_sizes]\nleft = 240\n[filetypes.typ]\nshow_console = true\n",
        )
        .unwrap();
        let md = crate::filetype::FileType::Md.pane_sizes(&s);
        assert_eq!(md.as_array(), [240, 460, 260]);
        assert_eq!(
            crate::filetype::FileType::Typ.pane_sizes(&s),
            PaneSizes::default()
        );
        // ダイアログの保存は大きさと開き方を保つ
        let kept = PanesOverride::explicit(Panes::default(), s.filetypes.md.clone());
        assert_eq!(kept.pane_sizes.left, 240);
        assert_eq!(kept.show_outline, Some(false));
    }

    #[test]
    fn instance_mode_is_lowercase_string() {
        let s: Settings = toml::from_str("[instance]\nmode = \"multiple\"\n").unwrap();
        assert_eq!(s.instance.mode, InstanceMode::Multiple);
        assert_eq!(Settings::default().instance.mode, InstanceMode::Single);
        let text = toml::to_string(&Settings::default()).unwrap();
        assert!(text.contains("mode = \"single\""), "{text}");
    }

    #[test]
    fn template_is_optional_and_multiline() {
        let s: Settings =
            toml::from_str("[filetypes.typ]\ntemplate = \"\"\"= 題名\n\n本文\"\"\"\n").unwrap();
        assert_eq!(s.filetypes.typ.template.as_deref(), Some("= 題名\n\n本文"));
        assert_eq!(s.filetypes.md.template, None);
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.filetypes.typ.template, s.filetypes.typ.template);
        // None は書き出さない
        assert!(!toml::to_string(&Settings::default())
            .unwrap()
            .contains("template"));
    }

    #[test]
    fn roundtrip() {
        let mut s = Settings::default();
        s.general.language = "ja".into();
        s.window.x = Some(10);
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }
}

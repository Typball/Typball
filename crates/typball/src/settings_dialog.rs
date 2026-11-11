//! 設定ダイアログ
//!
//! 左にページ一覧 (Treebook)、右に各ページの内容、下に OK / キャンセル。
//! ページごとに「ウィジェットを作る」「OK 時にウィジェットから `Settings` へ値を読み戻す」
//! の 2 つを書けば項目を増やせる。読み戻しは `Readers` にクロージャとして登録する。
//! ページの中身は最初に表示されるときに作る ([`PAGES`])。ネイティブのコンボボックスなどは
//! 1 つ作るのに数十 ms かかるので、全ページぶんを先に作るとダイアログが開くまで待たされる。

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::id::{ID_CANCEL, ID_HIGHEST, ID_OK};
use wxdragon::prelude::*;

use crate::assoc;
use crate::fileio::{Encoding, LineEnding};
use crate::filetype::FileType;
use crate::i18n::{self, tr, tr_args};
use crate::settings::{
    self, Backspace, BottomWidth, Channel, ExternalChange, InstanceMode, LastTab, LineNumbers,
    MarkdownMode, OpenMode, Panes, PanesOverride, RulerTabs, SaveMethod, SavedChanges, Settings,
    Theme, Wrap,
};

/// OK が押されたときに呼ばれる読み戻し処理の集まり
type Readers = Vec<Box<dyn Fn(&mut Settings)>>;

/// ページの中身を `panel` の中に作り、読み戻し処理を `readers` に登録する
type PageBuilder = fn(&Panel, &Settings, &mut Readers, &Dialog);

/// ページの並び: (タイトルのキー, 中身を作る関数, 直前の親ページの子か)。
/// 子ページは Treebook で親の下にぶら下がる (ファイル → 形式ごとの設定)
const PAGES: [(&str, PageBuilder, bool); 10] = [
    ("settings.general.title", page_general, false),
    ("settings.display.title", page_display, false),
    ("settings.editor.title", page_editor, false),
    ("settings.files.title", page_files, false),
    ("filetype.txt", page_filetype_txt, true),
    ("filetype.md", page_filetype_md, true),
    ("filetype.typ", page_filetype_typ, true),
    ("settings.preview.title", page_preview, false),
    ("settings.assoc.title", page_assoc, false),
    ("settings.update.title", page_update, false),
];

const LABEL_FLAGS: SizerFlag = SizerFlag::AlignCenterVertical;
const GAP: i32 = 8;

/// 「更新を今すぐ確認」で閉じたときの戻り値 (OK と同じく設定を読み戻す)
const ID_CHECK_UPDATE: i32 = ID_HIGHEST + 400;

/// OK で閉じたときの結果
pub struct Outcome {
    pub settings: Settings,
    /// 「更新を今すぐ確認」で閉じた (設定を反映してから確認する)
    pub check_update: bool,
}

/// ダイアログを開き、OK なら新しい設定を返す (キャンセルなら None)
pub fn show(parent: &Frame, current: &Settings) -> Option<Outcome> {
    let dialog = Dialog::builder(parent, &tr("settings.title"))
        .with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
        .build();
    // 寸法は DIP で書き、ダイアログのある画面の拡大率で実ピクセルにする
    let px = |v: i32| dialog.from_dip_int(v);

    let book = Treebook::builder(&dialog).build();
    let readers: Rc<RefCell<Readers>> = Rc::new(RefCell::new(Vec::new()));

    // ページは空のパネルだけ先に登録し、中身は最初に表示されるときに作る。
    // 一度も表示されなかったページの設定はそのまま残る (読み戻し処理が無い)
    let pages: Vec<Panel> = PAGES
        .iter()
        .map(|(title_key, _, sub)| {
            let panel = Panel::builder(&book).build();
            if *sub {
                book.add_sub_page(&panel, &tr(title_key), false, -1);
            } else {
                book.add_page(&panel, &tr(title_key), false, -1);
            }
            panel
        })
        .collect();
    let build_page = {
        let dialog = dialog.clone();
        let current = current.clone();
        let readers = readers.clone();
        let built = RefCell::new(vec![false; PAGES.len()]);
        Rc::new(move |index: usize| {
            let Some((panel, (_, build, _))) = pages.get(index).zip(PAGES.get(index)) else {
                return;
            };
            if std::mem::replace(&mut built.borrow_mut()[index], true) {
                return;
            }
            build(panel, &current, &mut readers.borrow_mut(), &dialog);
            panel.layout();
        })
    };
    // 子ページが最初から見えるように、いったん最後の子ページを選んで親を展開させてから戻す
    // (wxDragon の Treebook に ExpandNode が無い。選択はツリーの EnsureVisible で親を開く)
    if let Some(last_sub) = PAGES.iter().rposition(|(_, _, sub)| *sub) {
        book.set_selection(last_sub);
    }
    build_page(0);
    book.set_selection(0);
    // 切り替え「前」(PageChanging) に作る。切り替え後だとページが見えている状態で部品が
    // 左上 (0, 0) に作られ、並べ終わるまでの一瞬それが見える
    book.on_page_changing({
        let build_page = build_page.clone();
        move |event| {
            if let Some(i) = event.get_selection() {
                build_page(i.max(0) as usize);
            }
        }
    });

    // 設定ファイルの場所 (移行時に探しやすいよう常に表示する)。クリックでそのフォルダを開く
    let location = StaticText::builder(&dialog)
        .with_label(&tr_args(
            "settings.file_location",
            &[("path", &settings::path().display().to_string())],
        ))
        .build();
    location.set_tooltip(&tr("settings.file_location_tip"));
    if let Some(hand) = Cursor::from_stock(StockCursor::Hand) {
        location.set_cursor(Some(&hand));
    }
    location.on_mouse_left_up(|event| {
        open_settings_folder();
        event.skip(true);
    });

    let ok = Button::builder(&dialog)
        .with_id(ID_OK)
        .with_label(&tr("settings.ok"))
        .build();
    let cancel = Button::builder(&dialog)
        .with_id(ID_CANCEL)
        .with_label(&tr("settings.cancel"))
        .build();
    let buttons = StdDialogButtonSizerBuilder::new().build();
    buttons.add_button(&ok);
    buttons.add_button(&cancel);
    buttons.realize();

    let root = BoxSizer::builder(Orientation::Vertical).build();
    root.add(&book, 1, SizerFlag::Expand | SizerFlag::All, px(GAP));
    root.add(
        &location,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
        px(GAP),
    );
    root.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    dialog.set_sizer(root, true);
    // 長いパス (ポータブル運用など) はダイアログ幅で折り返す
    location.wrap(px(640));
    dialog.set_min_size(dialog.from_dip(Size::new(640, 480)));
    dialog.set_size(dialog.from_dip(Size::new(760, 520)));
    dialog.centre();
    // 見せる前に並べておく。wxMSW の wxDialog::Show は ::ShowWindow() の後に自分で WM_SIZE を
    // 送ってサイザーを動かすので、ここで並べておかないと部品が左上に固まった状態が先に見える
    dialog.layout();

    let result = dialog.show_modal();
    let out = if result == ID_OK || result == ID_CHECK_UPDATE {
        let mut s = current.clone();
        for read in readers.borrow().iter() {
            read(&mut s);
        }
        Some(Outcome {
            settings: s,
            check_update: result == ID_CHECK_UPDATE,
        })
    } else {
        None
    };
    dialog.destroy();
    out
}

/// 設定ファイルのあるフォルダをファイルマネージャで開く (ファイルがあれば選んだ状態で)。
/// まだ一度も保存していなければフォルダを作ってから開く
fn open_settings_folder() {
    let path = settings::path();
    if path.is_file() {
        crate::window::reveal_in_file_manager(&path);
        return;
    }
    let Some(dir) = path.parent() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("Typball: could not create {}: {e}", dir.display());
        return;
    }
    crate::window::open_in_file_manager(dir);
}

/// ラベル + ウィジェットの 2 列グリッドを作る
fn form_grid(panel: &Panel) -> FlexGridSizer {
    let px = |v: i32| panel.from_dip_int(v);
    let grid = FlexGridSizer::builder(0, 2)
        .with_vgap(px(GAP))
        .with_hgap(px(GAP * 2))
        .build();
    grid.add_growable_col(1, 1);
    grid
}

fn label(parent: &Panel, key: &str) -> StaticText {
    StaticText::builder(parent).with_label(&tr(key)).build()
}

/// フォントサイズの入力欄と、その右の「既定のフォント・サイズに戻す」ボタンを並べた行。
/// ボタンはフォント名とサイズの両方を既定値 (`face` / `size`) に戻す (エディタとプレビューで共通)
fn font_size_row(
    panel: &Panel,
    font_face: ComboBox,
    font_size: SpinCtrl,
    face: String,
    size: i32,
) -> BoxSizer {
    let reset = Button::builder(panel)
        .with_label(&tr("settings.editor.font_reset"))
        .build();
    reset.on_click(move |_| {
        font_face.set_value(&face);
        font_size.set_value(size);
    });
    let row = BoxSizer::builder(Orientation::Horizontal).build();
    row.add(&font_size, 0, SizerFlag::AlignCenterVertical, 0);
    row.add(
        &reset,
        0,
        SizerFlag::AlignCenterVertical | SizerFlag::Left,
        panel.from_dip_int(GAP),
    );
    row
}

/// 選択肢を入れた Choice を作る。項目は描画を止めてから入れる (wx は項目を 1 つ入れるたびに
/// ドロップダウンの高さを計算し直してサイズを変えるので、数項目でも数十 ms かかる)
fn choice_with(parent: &Panel, items: Vec<String>) -> Choice {
    let choice = Choice::builder(parent).build();
    choice.freeze();
    for item in &items {
        choice.append(item);
    }
    choice.thaw();
    choice
}

/// 配色 (システムに従う / ライト / ダーク) の行をグリッドに足し、OK 時の読み戻しを登録する
fn add_theme_row(
    panel: &Panel,
    grid: &FlexGridSizer,
    label_key: &str,
    current: Theme,
    readers: &mut Readers,
    store: fn(&mut Settings, Theme),
) {
    let choice = choice_with(
        panel,
        Theme::ALL.iter().map(|v| tr(theme_key(*v))).collect(),
    );
    let selected = Theme::ALL.iter().position(|v| *v == current).unwrap_or(0);
    choice.set_selection(selected as u32);
    grid.add(&label(panel, label_key), 0, LABEL_FLAGS, 0);
    grid.add(&choice, 0, SizerFlag::AlignLeft, 0);
    readers.push(Box::new(move |s| {
        let idx = choice.get_selection().unwrap_or(0) as usize;
        store(s, Theme::ALL.get(idx).copied().unwrap_or_default());
    }));
}

fn theme_key(value: Theme) -> &'static str {
    match value {
        Theme::System => "settings.theme.system",
        Theme::Light => "settings.theme.light",
        Theme::Dark => "settings.theme.dark",
    }
}

// ---------------------------------------------------------------------------
// 一般
// ---------------------------------------------------------------------------

fn page_general(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    let px = |v: i32| panel.from_dip_int(v);
    let grid = form_grid(panel);

    // 表示言語: 先頭が "システムに従う"、以降は locales/ の言語
    let languages = i18n::available();
    let mut choices = vec![tr("settings.general.language_system")];
    choices.extend(languages.iter().map(|(_, name)| name.clone()));
    let choice = choice_with(panel, choices);
    let selected = languages
        .iter()
        .position(|(code, _)| *code == current.general.language)
        .map(|i| i as u32 + 1)
        .unwrap_or(0);
    choice.set_selection(selected);

    grid.add(
        &label(panel, "settings.general.language"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&choice, 0, SizerFlag::Expand, 0);

    let note = StaticText::builder(panel)
        .with_label(&tr("settings.general.language_note"))
        .build();

    // 最後のタブを閉じたとき
    let last_tab = choice_with(
        panel,
        LastTab::ALL.iter().map(|v| tr(last_tab_key(*v))).collect(),
    );
    let selected = LastTab::ALL
        .iter()
        .position(|v| *v == current.general.last_tab)
        .unwrap_or(0);
    last_tab.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.general.last_tab"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&last_tab, 0, SizerFlag::AlignLeft, 0);

    // 別に起動したとき (シングルインスタンス)
    let instance = choice_with(
        panel,
        InstanceMode::ALL
            .iter()
            .map(|v| tr(instance_mode_key(*v)))
            .collect(),
    );
    let selected = InstanceMode::ALL
        .iter()
        .position(|v| *v == current.instance.mode)
        .unwrap_or(0);
    instance.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.general.instance"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&instance, 0, SizerFlag::AlignLeft, 0);

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    sizer.add(&note, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    panel.set_sizer(sizer, true);
    note.wrap(px(360));

    readers.push(Box::new(move |s| {
        let idx = choice.get_selection().unwrap_or(0) as usize;
        s.general.language = if idx == 0 {
            i18n::SYSTEM.to_string()
        } else {
            languages
                .get(idx - 1)
                .map(|(code, _)| code.to_string())
                .unwrap_or_else(|| i18n::SYSTEM.to_string())
        };
        let idx = last_tab.get_selection().unwrap_or(0) as usize;
        s.general.last_tab = LastTab::ALL.get(idx).copied().unwrap_or_default();
        let idx = instance.get_selection().unwrap_or(0) as usize;
        s.instance.mode = InstanceMode::ALL.get(idx).copied().unwrap_or_default();
    }));
}

fn instance_mode_key(value: InstanceMode) -> &'static str {
    match value {
        InstanceMode::Single => "settings.general.instance_single",
        InstanceMode::Multiple => "settings.general.instance_multiple",
    }
}

fn last_tab_key(value: LastTab) -> &'static str {
    match value {
        LastTab::KeepEmpty => "settings.general.last_tab_keep",
        LastTab::CloseWindow => "settings.general.last_tab_close",
    }
}

// ---------------------------------------------------------------------------
// 表示 (配色・ペインの配置)
// ---------------------------------------------------------------------------

fn page_display(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    let px = |v: i32| panel.from_dip_int(v);
    let grid = form_grid(panel);

    // ウィンドウ (メニュー・タブ・ダイアログ)・エディタ・プレビューの配色
    add_theme_row(
        panel,
        &grid,
        "settings.display.window_theme",
        current.general.theme,
        readers,
        |s, v| s.general.theme = v,
    );
    add_theme_row(
        panel,
        &grid,
        "settings.display.editor_theme",
        current.editor.theme,
        readers,
        |s, v| s.editor.theme = v,
    );
    add_theme_row(
        panel,
        &grid,
        "settings.display.preview_theme",
        current.preview.theme,
        readers,
        |s, v| s.preview.theme = v,
    );

    // 下のペインの幅
    let bottom_width = choice_with(
        panel,
        BottomWidth::ALL
            .iter()
            .map(|v| tr(bottom_width_key(*v)))
            .collect(),
    );
    let selected = BottomWidth::ALL
        .iter()
        .position(|v| *v == current.panes.bottom_width)
        .unwrap_or(0);
    bottom_width.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.display.bottom_width"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&bottom_width, 0, SizerFlag::AlignLeft, 0);

    // ツールバー (形式によらず全ウィンドウで共通)
    let show_toolbar = CheckBox::builder(panel)
        .with_label(&tr("settings.display.show_toolbar"))
        .with_value(current.general.show_toolbar)
        .build();

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    sizer.add(
        &show_toolbar,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
        px(GAP),
    );
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        let idx = bottom_width.get_selection().unwrap_or(0) as usize;
        s.panes.bottom_width = BottomWidth::ALL.get(idx).copied().unwrap_or_default();
        s.general.show_toolbar = show_toolbar.is_checked();
    }));
}

fn bottom_width_key(value: BottomWidth) -> &'static str {
    match value {
        BottomWidth::Editor => "settings.display.bottom_width_editor",
        BottomWidth::Window => "settings.display.bottom_width_window",
    }
}

// ---------------------------------------------------------------------------
// エディタ
// ---------------------------------------------------------------------------

fn page_editor(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    let px = |v: i32| panel.from_dip_int(v);
    let grid = form_grid(panel);

    // 同梱の UDEV Gothic (既定) を先頭に、続いて等幅フォント、残りを後に (どちらも名前順)。手入力もできる
    let embedded = crate::fonts::EDITOR_FONT_FAMILY;
    let families = typball_core::system_font_families();
    let mut choices: Vec<String> = vec![embedded.to_string()];
    choices.extend(
        families
            .iter()
            .filter(|f| f.monospace)
            .map(|f| f.name.clone())
            .filter(|name| name != embedded),
    );
    choices.extend(
        families
            .iter()
            .filter(|f| !f.monospace)
            .map(|f| f.name.clone()),
    );
    // 一覧は最初にドロップダウンを開いたときに入れる (数百のフォント名を入れるのに数百 ms
    // かかり、ダイアログが開くのが遅くなる)。まとめて追加し、描画も止めておく
    let font_face = ComboBox::builder(panel)
        .with_value(&current.editor.font_face)
        .build();
    let filled = std::rc::Rc::new(std::cell::Cell::new(false));
    font_face.on_dropdown({
        let filled = filled.clone();
        move |_| {
            if filled.replace(true) {
                return;
            }
            let refs: Vec<&str> = choices.iter().map(|s| s.as_str()).collect();
            font_face.freeze();
            font_face.append_items(&refs);
            font_face.thaw();
        }
    });
    grid.add(
        &label(panel, "settings.editor.font_face"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&font_face, 0, SizerFlag::Expand, 0);

    let font_size = SpinCtrl::builder(panel)
        .with_range(6, 72)
        .with_initial_value(current.editor.font_size)
        .build();
    grid.add(
        &label(panel, "settings.editor.font_size"),
        0,
        LABEL_FLAGS,
        0,
    );
    let defaults = settings::Editor::default();
    grid.add_sizer(
        &font_size_row(
            panel,
            font_face,
            font_size,
            defaults.font_face,
            defaults.font_size,
        ),
        0,
        SizerFlag::AlignLeft,
        0,
    );

    // 行間: 1 行の高さに足す px
    let line_spacing = SpinCtrl::builder(panel)
        .with_range(0, 40)
        .with_initial_value(current.editor.line_spacing)
        .build();
    grid.add(
        &label(panel, "settings.editor.line_spacing"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&line_spacing, 0, SizerFlag::AlignLeft, 0);

    let tab_width = SpinCtrl::builder(panel)
        .with_range(1, 16)
        .with_initial_value(current.editor.tab_width)
        .build();
    grid.add(
        &label(panel, "settings.editor.tab_width"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&tab_width, 0, SizerFlag::AlignLeft, 0);

    // Tab キーで入力するもの: スペース (タブ位置まで) / タブ文字
    let tab_key = choice_with(
        panel,
        vec![
            tr("settings.editor.tab_key_spaces"),
            tr("settings.editor.tab_key_tab"),
        ],
    );
    tab_key.set_selection(current.editor.use_tabs as u32);
    grid.add(&label(panel, "settings.editor.tab_key"), 0, LABEL_FLAGS, 0);
    grid.add(&tab_key, 0, SizerFlag::AlignLeft, 0);

    // Backspace でスペースを消すときの動作: Backspace::ALL の並び順
    let backspace = choice_with(
        panel,
        Backspace::ALL
            .iter()
            .map(|mode| {
                tr(match mode {
                    Backspace::Char => "settings.editor.backspace_char",
                    Backspace::SoftTab => "settings.editor.backspace_soft_tab",
                    Backspace::TabStop => "settings.editor.backspace_tab_stop",
                    Backspace::FullTabStop => "settings.editor.backspace_full_tab_stop",
                    Backspace::ToTabStop => "settings.editor.backspace_to_tab_stop",
                    Backspace::Indent => "settings.editor.backspace_indent",
                })
            })
            .collect(),
    );
    let selected = Backspace::ALL
        .iter()
        .position(|mode| *mode == current.editor.backspace)
        .unwrap_or(0);
    backspace.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.editor.backspace"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&backspace, 0, SizerFlag::AlignLeft, 0);

    // 折り返し: Wrap::ALL の並び順。文字数は「指定文字数で折り返す」のときだけ使う
    let wrap = choice_with(panel, Wrap::ALL.iter().map(|v| tr(wrap_key(*v))).collect());
    let selected = Wrap::ALL
        .iter()
        .position(|v| *v == current.editor.wrap)
        .unwrap_or(1);
    wrap.set_selection(selected as u32);
    grid.add(&label(panel, "settings.editor.wrap"), 0, LABEL_FLAGS, 0);
    grid.add(&wrap, 0, SizerFlag::AlignLeft, 0);

    let wrap_column = SpinCtrl::builder(panel)
        .with_range(10, 1000)
        .with_initial_value(current.editor.wrap_column)
        .build();
    grid.add(
        &label(panel, "settings.editor.wrap_column_width"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&wrap_column, 0, SizerFlag::AlignLeft, 0);

    // ルーラー: 表示しない / 表示する (タブ位置の示し方 = RulerTabs::ALL の並び順)
    let mut ruler_labels = vec![tr("settings.editor.ruler_off")];
    ruler_labels.extend(RulerTabs::ALL.iter().map(|tabs| {
        tr(match tabs {
            RulerTabs::None => "settings.editor.ruler_plain",
            RulerTabs::Arrow => "settings.editor.ruler_arrow",
        })
    }));
    let ruler = choice_with(panel, ruler_labels);
    let selected = if current.editor.show_ruler {
        1 + RulerTabs::ALL
            .iter()
            .position(|tabs| *tabs == current.editor.ruler_tabs)
            .unwrap_or(0)
    } else {
        0
    };
    ruler.set_selection(selected as u32);
    grid.add(&label(panel, "settings.editor.ruler"), 0, LABEL_FLAGS, 0);
    grid.add(&ruler, 0, SizerFlag::AlignLeft, 0);

    // 行番号の位置: LineNumbers::ALL の並び順で選択肢を作る
    let line_numbers = choice_with(
        panel,
        LineNumbers::ALL
            .iter()
            .map(|v| tr(line_numbers_key(*v)))
            .collect(),
    );
    let selected = LineNumbers::ALL
        .iter()
        .position(|v| *v == current.editor.line_numbers)
        .unwrap_or(1);
    line_numbers.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.editor.line_numbers"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&line_numbers, 0, SizerFlag::AlignLeft, 0);

    // 変更行の強調: 強調するか (highlight_changes) と保存したときの扱い (saved_changes) を
    // 1 つの選択肢にまとめる。強調しないときは保存時の扱いを今の値のまま残す
    // (メニューの「表示 → 編集した行を強調」で入れ直したときに元の扱いに戻るように)
    let change_highlight = choice_with(
        panel,
        CHANGE_HIGHLIGHT.iter().map(|(key, _)| tr(key)).collect(),
    );
    let selected = if current.editor.highlight_changes {
        CHANGE_HIGHLIGHT
            .iter()
            .position(|(_, v)| *v == Some(current.editor.saved_changes))
            .unwrap_or(1)
    } else {
        0
    };
    change_highlight.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.editor.change_highlight"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&change_highlight, 0, SizerFlag::AlignLeft, 0);

    // 不可視文字の記号 (表示 > 記号を表示 と同じ)。1 行に並べる (縦に積むとページに収まらない)
    let mark_boxes: Vec<CheckBox> = [
        ("settings.editor.show_eol", current.editor.show_eol),
        ("settings.editor.show_eof", current.editor.show_eof),
        (
            "settings.editor.show_whitespace",
            current.editor.show_whitespace,
        ),
    ]
    .into_iter()
    .map(|(key, value)| {
        CheckBox::builder(panel)
            .with_label(&tr(key))
            .with_value(value)
            .build()
    })
    .collect();
    let marks_row = BoxSizer::builder(Orientation::Horizontal).build();
    for check in &mark_boxes {
        marks_row.add(
            check,
            0,
            SizerFlag::AlignCenterVertical | SizerFlag::Right,
            px(GAP * 2),
        );
    }
    grid.add(
        &label(panel, "settings.editor.show_marks"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add_sizer(&marks_row, 0, SizerFlag::AlignLeft, 0);

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        s.editor.font_face = font_face.get_value().trim().to_string();
        s.editor.font_size = font_size.value();
        s.editor.tab_width = tab_width.value();
        s.editor.line_spacing = line_spacing.value().clamp(0, 40);
        s.editor.use_tabs = tab_key.get_selection() == Some(1);
        s.editor.backspace = backspace
            .get_selection()
            .and_then(|i| Backspace::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.editor.wrap = wrap
            .get_selection()
            .and_then(|i| Wrap::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.editor.wrap_column = wrap_column.value();
        s.editor.line_numbers = line_numbers
            .get_selection()
            .and_then(|i| LineNumbers::ALL.get(i as usize).copied())
            .unwrap_or_default();
        let idx = change_highlight.get_selection().unwrap_or(1) as usize;
        match CHANGE_HIGHLIGHT.get(idx).and_then(|(_, v)| *v) {
            Some(saved) => {
                s.editor.highlight_changes = true;
                s.editor.saved_changes = saved;
            }
            None => s.editor.highlight_changes = false,
        }
        // 「表示しない」を選んでも、タブ位置の示し方は残す (表示メニューで出し直したときに戻る)
        match ruler.get_selection().unwrap_or(0) as usize {
            0 => s.editor.show_ruler = false,
            i => {
                s.editor.show_ruler = true;
                s.editor.ruler_tabs = RulerTabs::ALL.get(i - 1).copied().unwrap_or_default();
            }
        }
        s.editor.show_eol = mark_boxes[0].is_checked();
        s.editor.show_eof = mark_boxes[1].is_checked();
        s.editor.show_whitespace = mark_boxes[2].is_checked();
    }));
}

fn wrap_key(value: Wrap) -> &'static str {
    match value {
        Wrap::None => "settings.editor.wrap_none",
        Wrap::Window => "settings.editor.wrap_window",
        Wrap::Column => "settings.editor.wrap_column",
    }
}

/// 「変更行の強調」の選択肢: (表示名のキー, 保存したときの扱い)。None は強調しない
const CHANGE_HIGHLIGHT: [(&str, Option<SavedChanges>); 3] = [
    ("settings.editor.change_highlight_off", None),
    (
        "settings.editor.change_highlight_keep",
        Some(SavedChanges::Keep),
    ),
    (
        "settings.editor.change_highlight_clear",
        Some(SavedChanges::Clear),
    ),
];

// ---------------------------------------------------------------------------
// ファイル
// ---------------------------------------------------------------------------

fn page_files(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    let px = |v: i32| panel.from_dip_int(v);
    let grid = form_grid(panel);

    // 新規タブの形式
    let default_type = choice_with(
        panel,
        FileType::ALL.iter().map(|t| tr(t.name_key())).collect(),
    );
    let current_type = FileType::from_extension(&current.files.default_filetype);
    let selected = FileType::ALL
        .iter()
        .position(|t| *t == current_type)
        .unwrap_or(0);
    default_type.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.default_filetype"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&default_type, 0, SizerFlag::AlignLeft, 0);

    // 外部で変更されたとき
    let external = choice_with(
        panel,
        ExternalChange::ALL
            .iter()
            .map(|v| tr(external_change_key(*v)))
            .collect(),
    );
    let selected = ExternalChange::ALL
        .iter()
        .position(|v| *v == current.files.external_change)
        .unwrap_or(0);
    external.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.external_change"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&external, 0, SizerFlag::AlignLeft, 0);

    // 保存のしかた: 一時ファイルと入れ替える / 直接書き込む
    let save_method = choice_with(
        panel,
        SaveMethod::ALL
            .iter()
            .map(|v| tr(save_method_key(*v)))
            .collect(),
    );
    let selected = SaveMethod::ALL
        .iter()
        .position(|v| *v == current.files.save_method)
        .unwrap_or(0);
    save_method.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.save_method"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&save_method, 0, SizerFlag::AlignLeft, 0);

    // 文字コード: 新規文書の既定と、UTF-8 として読めないときの既定 (先頭は「推定」)
    let encodings = Encoding::all();
    let labels: Vec<String> = encodings.iter().map(|e| e.label()).collect();
    let default_encoding = choice_with(panel, labels.clone());
    let selected = Encoding::parse(&current.files.default_encoding)
        .and_then(|e| encodings.iter().position(|x| *x == e))
        .unwrap_or(0);
    default_encoding.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.default_encoding"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&default_encoding, 0, SizerFlag::AlignLeft, 0);

    // 新規文書の改行コード (先頭は「OS の標準」)
    let mut eol_labels = vec![tr_args(
        "settings.files.line_ending_auto",
        &[("eol", LineEnding::platform().short_label())],
    )];
    eol_labels.extend(LineEnding::ALL.iter().map(|e| tr(e.label_key())));
    let default_line_ending = choice_with(panel, eol_labels);
    let selected = LineEnding::parse(&current.files.default_line_ending)
        .and_then(|e| LineEnding::ALL.iter().position(|x| *x == e))
        .map(|i| i + 1)
        .unwrap_or(0);
    default_line_ending.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.default_line_ending"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&default_line_ending, 0, SizerFlag::AlignLeft, 0);

    let mut fallback_labels = vec![tr("settings.files.fallback_auto")];
    fallback_labels.extend(labels);
    let fallback_encoding = choice_with(panel, fallback_labels);
    let selected = Encoding::parse(&current.files.fallback_encoding)
        .and_then(|e| encodings.iter().position(|x| *x == e))
        .map(|i| i + 1)
        .unwrap_or(0);
    fallback_encoding.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.fallback_encoding"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&fallback_encoding, 0, SizerFlag::AlignLeft, 0);

    // 大きなファイルの扱い (MB)。ラベルが長いので別の表にして、上の選択肢をその幅で右へ押し出さない
    let size_grid = form_grid(panel);
    let large_mb = SpinCtrl::builder(panel)
        .with_range(0, 100_000)
        .with_initial_value(current.files.large_file_mb as i32)
        .build();
    size_grid.add(
        &label(panel, "settings.files.large_file_mb"),
        0,
        LABEL_FLAGS,
        0,
    );
    size_grid.add(&large_mb, 0, SizerFlag::AlignLeft, 0);
    let max_mb = SpinCtrl::builder(panel)
        .with_range(0, 100_000)
        .with_initial_value(current.files.max_file_mb as i32)
        .build();
    size_grid.add(
        &label(panel, "settings.files.max_file_mb"),
        0,
        LABEL_FLAGS,
        0,
    );
    size_grid.add(&max_mb, 0, SizerFlag::AlignLeft, 0);

    // 新規文書の番号
    let numbering = CheckBox::builder(panel)
        .with_label(&tr("settings.files.untitled_numbering"))
        .with_value(current.files.untitled_numbering)
        .build();
    // 未保存の文書のスナップショット
    let recovery = CheckBox::builder(panel)
        .with_label(&tr("settings.files.recovery_backup"))
        .with_value(current.files.recovery_backup)
        .build();

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    sizer.add_sizer(
        &size_grid,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(GAP),
    );
    sizer.add(&numbering, 0, SizerFlag::Left | SizerFlag::Right, px(GAP));
    sizer.add(&recovery, 0, SizerFlag::All, px(GAP));
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        s.files.default_filetype = default_type
            .get_selection()
            .and_then(|i| FileType::ALL.get(i as usize))
            .map(|t| t.extension().to_string())
            .unwrap_or_else(|| "txt".to_string());
        s.files.external_change = external
            .get_selection()
            .and_then(|i| ExternalChange::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.files.save_method = save_method
            .get_selection()
            .and_then(|i| SaveMethod::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.files.untitled_numbering = numbering.is_checked();
        s.files.default_encoding = default_encoding
            .get_selection()
            .and_then(|i| encodings.get(i as usize))
            .map(|e| e.id())
            .unwrap_or_else(|| "utf-8".to_string());
        s.files.default_line_ending = default_line_ending
            .get_selection()
            .and_then(|i| (i as usize).checked_sub(1))
            .and_then(|i| LineEnding::ALL.get(i))
            .map(|e| e.id().to_string())
            .unwrap_or_else(|| "auto".to_string());
        s.files.fallback_encoding = fallback_encoding
            .get_selection()
            .filter(|i| *i > 0)
            .and_then(|i| encodings.get(i as usize - 1))
            .map(|e| e.id())
            .unwrap_or_else(|| "auto".to_string());
        s.files.large_file_mb = large_mb.value().max(0) as u32;
        s.files.max_file_mb = max_mb.value().max(0) as u32;
        s.files.recovery_backup = recovery.is_checked();
    }));
}

// ---------------------------------------------------------------------------
// ファイル → テキスト / Markdown / Typst: 形式ごとのペイン・開き方・雛形
// ---------------------------------------------------------------------------

fn page_filetype_txt(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    page_filetype(panel, current, readers, FileType::Txt);
}

fn page_filetype_md(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    page_filetype(panel, current, readers, FileType::Md);
}

fn page_filetype_typ(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    page_filetype(panel, current, readers, FileType::Typ);
}

/// 形式ごとの設定: 開いたときのペインとバー、スクロール同期 (対応する形式だけ)、
/// ファイルを渡されたときの開き方 (プレビューのある形式だけ)、新しいファイルの雛形
fn page_filetype(panel: &Panel, current: &Settings, readers: &mut Readers, ft: FileType) {
    let px = |v: i32| panel.from_dip_int(v);
    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    // チェックボックスを横に並べる行
    let row = |boxes: &[&CheckBox]| {
        let row = BoxSizer::builder(Orientation::Horizontal).build();
        for (i, cb) in boxes.iter().enumerate() {
            let gap = if i == 0 { 0 } else { px(GAP * 2) };
            row.add(*cb, 0, SizerFlag::Left | LABEL_FLAGS, gap);
        }
        row
    };
    let check = |key: &str, value: bool, enabled: bool| {
        let cb = CheckBox::builder(panel)
            .with_label(&tr(key))
            .with_value(value && enabled)
            .build();
        cb.enable(enabled);
        cb
    };

    // 開いたときのペインとバー (この形式に無いものは選べない)
    let panes = ft.panes(current);
    let outline = check(
        "settings.files.show_outline",
        panes.show_outline,
        ft.has_outline(),
    );
    let preview = check(
        "settings.files.show_preview",
        panes.show_preview,
        ft.has_preview(),
    );
    let console = check(
        "settings.files.show_console",
        panes.show_console,
        ft.has_console(),
    );
    let toolbar = check(
        "settings.files.show_actionbar",
        panes.show_actionbar,
        ft.has_actionbar(),
    );
    let panes_note = StaticText::builder(panel)
        .with_label(&tr("settings.files.panes_note"))
        .build();
    sizer.add(
        &panes_note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
        px(GAP),
    );
    sizer.add_sizer(
        &row(&[&outline, &preview, &console, &toolbar]),
        0,
        SizerFlag::All,
        px(GAP),
    );

    // スクロール同期 (方向ごと)
    let sync = ft.has_scroll_sync().then(|| {
        let to_preview = check(
            "settings.files.sync_to_preview",
            panes.sync_scroll_to_preview,
            true,
        );
        let to_editor = check(
            "settings.files.sync_to_editor",
            panes.sync_scroll_to_editor,
            true,
        );
        let note = StaticText::builder(panel)
            .with_label(&tr("settings.files.sync_note"))
            .build();
        sizer.add(
            &note,
            0,
            SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
            px(GAP),
        );
        sizer.add_sizer(&row(&[&to_preview, &to_editor]), 0, SizerFlag::All, px(GAP));
        (to_preview, to_editor)
    });

    // ファイルを渡されて起動したときの開き方
    let in_viewer = ft.has_preview().then(|| {
        let in_viewer = check(
            "settings.files.open_in_viewer",
            ft.open_mode(current) == OpenMode::Viewer,
            true,
        );
        let note = StaticText::builder(panel)
            .with_label(&tr("settings.files.open_note"))
            .build();
        sizer.add(
            &note,
            0,
            SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
            px(GAP),
        );
        sizer.add(&in_viewer, 0, SizerFlag::All, px(GAP));
        in_viewer
    });

    // 新しいファイルの雛形 (Ctrl+N と起動時の新規タブの中身)。複数行のテキストと「既定に戻す」
    let template_note = StaticText::builder(panel)
        .with_label(&tr_args(
            "settings.files.template_note",
            &[("name", &tr(ft.name_key()))],
        ))
        .build();
    let text = TextCtrl::builder(panel)
        .with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::DontWrap | TextCtrlStyle::Rich2)
        .build();
    text.set_value(&ft.template(current));
    if let Some(font) = Font::builder()
        .with_face_name(&current.editor.font_face)
        .with_point_size(current.editor.font_size.clamp(6, 32))
        .build()
    {
        text.set_font(&font);
    }
    let reset = Button::builder(panel)
        .with_label(&tr("settings.files.template_reset"))
        .build();
    reset.on_click(move |_| text.set_value(&ft.default_template()));
    sizer.add(
        &template_note,
        0,
        SizerFlag::Expand | SizerFlag::All,
        px(GAP),
    );
    sizer.add(
        &text,
        1,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
        px(GAP),
    );
    sizer.add(&reset, 0, SizerFlag::AlignRight | SizerFlag::All, px(GAP));
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        // 開き方・ペインの大きさは今の値から始める。このページに無い項目 (同期できない形式の同期) も今の値のまま
        let now = ft.panes(s);
        let mut panes = PanesOverride::explicit(
            Panes {
                show_outline: outline.is_checked(),
                show_preview: preview.is_checked(),
                show_console: console.is_checked(),
                show_actionbar: toolbar.is_checked(),
                sync_scroll_to_preview: sync
                    .as_ref()
                    .map(|(cb, _)| cb.is_checked())
                    .unwrap_or(now.sync_scroll_to_preview),
                sync_scroll_to_editor: sync
                    .as_ref()
                    .map(|(_, cb)| cb.is_checked())
                    .unwrap_or(now.sync_scroll_to_editor),
            },
            ft.overrides(s),
        );
        if let Some(cb) = &in_viewer {
            panes.open_mode = if cb.is_checked() {
                OpenMode::Viewer
            } else {
                OpenMode::Editor
            };
        }
        let value = text.get_value();
        // 既定のままなら書き出さない (表示言語を変えたときに既定も切り替わるように)
        panes.template = (value != ft.default_template()).then_some(value);
        *ft.overrides_mut(s) = panes;
    }));
}

// ---------------------------------------------------------------------------
// プレビュー
// ---------------------------------------------------------------------------

fn page_preview(panel: &Panel, current: &Settings, readers: &mut Readers, _dialog: &Dialog) {
    let px = |v: i32| panel.from_dip_int(v);
    let grid = form_grid(panel);

    // プレビューのフォント (Typst の既定・Markdown の本文・プレビューウィンドウ)。同梱の Noto Sans JP を先頭に
    let embedded = typball_core::EMBEDDED_FONT_FAMILY;
    let mut choices: Vec<String> = vec![embedded.to_string()];
    choices.extend(
        typball_core::system_font_families()
            .iter()
            .map(|f| f.name.clone())
            .filter(|name| name != embedded),
    );
    let font_face = ComboBox::builder(panel)
        .with_value(&current.preview.font_face)
        .build();
    let filled = std::rc::Rc::new(std::cell::Cell::new(false));
    font_face.on_dropdown({
        let filled = filled.clone();
        move |_| {
            if filled.replace(true) {
                return;
            }
            let refs: Vec<&str> = choices.iter().map(|s| s.as_str()).collect();
            font_face.freeze();
            font_face.append_items(&refs);
            font_face.thaw();
        }
    });
    grid.add(
        &label(panel, "settings.preview.font_face"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&font_face, 0, SizerFlag::Expand, 0);
    let font_size = SpinCtrl::builder(panel)
        .with_range(6, 72)
        .with_initial_value(current.preview.font_size)
        .build();
    grid.add(
        &label(panel, "settings.preview.font_size"),
        0,
        LABEL_FLAGS,
        0,
    );
    let defaults = settings::Preview::default();
    grid.add_sizer(
        &font_size_row(
            panel,
            font_face,
            font_size,
            defaults.font_face,
            defaults.font_size,
        ),
        0,
        SizerFlag::AlignLeft,
        0,
    );
    let font_note = StaticText::builder(panel)
        .with_label(&tr("settings.preview.font_note"))
        .build();

    // Markdown のプレビュー方式
    let mode = choice_with(
        panel,
        MarkdownMode::ALL
            .iter()
            .map(|m| tr(markdown_mode_key(*m)))
            .collect(),
    );
    let selected = MarkdownMode::ALL
        .iter()
        .position(|m| *m == current.preview.markdown_mode)
        .unwrap_or(0);
    mode.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.preview.markdown_mode"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&mode, 0, SizerFlag::AlignLeft, 0);

    // 開発サーバーのコマンドと URL
    let command = TextCtrl::builder(panel).build();
    command.set_value(&current.preview.site_command);
    grid.add(
        &label(panel, "settings.preview.site_command"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&command, 0, SizerFlag::Expand, 0);
    let url = TextCtrl::builder(panel).build();
    url.set_value(&current.preview.site_url);
    grid.add(
        &label(panel, "settings.preview.site_url"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&url, 0, SizerFlag::Expand, 0);

    let note = StaticText::builder(panel)
        .with_label(&tr("settings.preview.site_note"))
        .build();

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    sizer.add(
        &font_note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(GAP),
    );
    sizer.add(
        &note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(GAP),
    );
    panel.set_sizer(sizer, true);
    // 折り返しはサイザーに入れてから (先に Wrap すると wxMSW で高さが折り返し前のまま残り、末尾の行が隠れる)
    font_note.wrap(px(520));
    note.wrap(px(520));
    panel.layout();

    readers.push(Box::new(move |s| {
        let face = font_face.get_value();
        s.preview.font_face = if face.trim().is_empty() {
            typball_core::EMBEDDED_FONT_FAMILY.to_string()
        } else {
            face.trim().to_string()
        };
        s.preview.font_size = font_size.value().clamp(6, 72);
        s.preview.markdown_mode = mode
            .get_selection()
            .and_then(|i| MarkdownMode::ALL.get(i as usize).copied())
            .unwrap_or_default();
        let defaults = crate::settings::Preview::default();
        let command = command.get_value();
        s.preview.site_command = if command.trim().is_empty() {
            defaults.site_command
        } else {
            command
        };
        let url = url.get_value();
        s.preview.site_url = if url.trim().is_empty() {
            defaults.site_url
        } else {
            url
        };
    }));
}

// ---------------------------------------------------------------------------
// 関連付け (OS への登録。Settings.toml には保存せず、操作はその場で OS に反映する)
// ---------------------------------------------------------------------------

/// 関連付けのアイコンの見本の一辺 (DIP)
const ASSOC_ICON_SIZE: i32 = 32;

/// 関連付けのページの 1 行 (1 形式ぶん) の部品
#[derive(Clone)]
struct AssocRow {
    kind: assoc::Kind,
    /// 行を載せているページ (表示を変えた後に並べ直す)
    panel: Panel,
    /// 「既定のアプリ: ...」
    default_text: StaticText,
    /// 「Typball: 登録済み」など
    registered_text: StaticText,
    unregister: Button,
    /// アイコンの見本と選択肢 (アイコンを選べる OS だけ)
    icon: Option<(StaticBitmap, Choice)>,
    /// 選んであるアイコン。未登録のうちは、次に関連付けるときに使う
    chosen: Rc<RefCell<assoc::Icon>>,
}

impl AssocRow {
    /// OS に今の状態を聞いて表示を合わせる
    fn refresh(&self) {
        let status = assoc::status(self.kind);
        let default_text = match &status.default_app {
            None => tr("settings.assoc.default_none"),
            Some(app) if app.is_self => {
                tr_args("settings.assoc.default_self", &[("name", &app.name)])
            }
            Some(app) => tr_args("settings.assoc.default_app", &[("name", &app.name)]),
        };
        // 長いパスで行が伸びないよう幅は固定してあるので、全文はツールチップで読めるようにする
        let default_tip = match status.default_app.as_ref().and_then(|a| a.path.as_ref()) {
            Some(path) => format!("{default_text}\n{}", path.display()),
            None => default_text.clone(),
        };
        self.default_text.set_label(&default_text);
        self.default_text.set_tooltip(&default_tip);

        match &status.registered {
            assoc::Registered::No => {
                set_path_label(&self.registered_text, "", |_| {
                    tr("settings.assoc.registered_no")
                });
            }
            assoc::Registered::This => {
                set_path_label(&self.registered_text, "", |_| {
                    tr("settings.assoc.registered_this")
                });
            }
            assoc::Registered::Other(path) => {
                set_path_label(&self.registered_text, &path.display().to_string(), |p| {
                    tr_args("settings.assoc.registered_other", &[("path", p)])
                });
            }
        }
        self.unregister.enable(status.removable);

        // 登録済みならそのアイコンを、未登録なら選びかけのアイコンを見せる
        if status.registered != assoc::Registered::No {
            *self.chosen.borrow_mut() = status.icon;
        }
        self.show_icon();
        // wxStaticText は文字を入れ替えると自分の大きさを文字に合わせ直すので、サイザーの決めた幅に戻す
        self.panel.layout();
    }

    /// 選んであるアイコンを見本と選択肢に反映する
    fn show_icon(&self) {
        let Some((preview, choice)) = &self.icon else {
            return;
        };
        let chosen = self.chosen.borrow().clone();
        let (index, tip) = match &chosen {
            assoc::Icon::Document => (0, String::new()),
            assoc::Icon::App => (1, String::new()),
            assoc::Icon::Custom(value) => (2, value.clone()),
        };
        choice.set_selection(index);
        choice.set_tooltip(&tip);
        let size = preview.from_dip_int(ASSOC_ICON_SIZE) as u32;
        // 取り出せないアイコン (消えたファイルなど) は空の見本にする
        let rgba = assoc::icon_rgba(self.kind, &chosen, size)
            .unwrap_or_else(|| vec![0; (size * size * 4) as usize]);
        if let Some(bitmap) = Bitmap::from_rgba(&rgba, size, size) {
            preview.set_bitmap(&bitmap);
        }
    }
}

/// OS に登録する文字列 (右クリックメニューの項目名) を今の表示言語で作る
fn assoc_labels() -> assoc::Labels {
    assoc::Labels {
        open: tr("settings.assoc.verb_open"),
        edit: tr("settings.assoc.verb_edit"),
        view: tr("settings.assoc.verb_view"),
    }
}

/// 関連付けの操作の結果を受けて、失敗なら知らせる
fn assoc_report(dialog: &Dialog, result: Result<(), String>) {
    if let Err(error) = result {
        MessageDialog::builder(
            dialog,
            &tr_args("settings.assoc.error", &[("error", &error)]),
            "Typball",
        )
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
        .build()
        .show_modal();
    }
}

fn page_assoc(panel: &Panel, _current: &Settings, _readers: &mut Readers, dialog: &Dialog) {
    let px = |v: i32| panel.from_dip_int(v);
    let caps = assoc::capabilities();
    let sizer = BoxSizer::builder(Orientation::Vertical).build();

    let intro = StaticText::builder(panel)
        .with_label(&tr("settings.assoc.intro"))
        .build();
    sizer.add(&intro, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));

    // 登録する Typball の場所 (開発版とインストール版のように複数あるとき、どれを登録するのかが分かるように)
    let target = assoc::target()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    // 見出しの次の行にパスを置き、下の形式の一覧とは間を空ける (1 行に続けると詰まって見える)
    sizer.add(
        &label(panel, "settings.assoc.target"),
        0,
        SizerFlag::Left | SizerFlag::Right,
        px(GAP),
    );
    // 文字は並べ終えてから入れる (幅に収まらないパスは途中を省略する)
    let target_label = StaticText::builder(panel).build();
    target_label.set_min_size(Size::new(px(120), -1));
    sizer.add_spacer(px(4));
    sizer.add(
        &target_label,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
        px(GAP),
    );
    sizer.add_spacer(px(GAP * 2));

    let rows: Rc<Vec<AssocRow>> = Rc::new(
        [
            (assoc::Kind::Txt, FileType::Txt),
            (assoc::Kind::Md, FileType::Md),
            (assoc::Kind::Typ, FileType::Typ),
        ]
        .into_iter()
        .map(|(kind, ft)| assoc_row(panel, &sizer, dialog, kind, ft, caps))
        .collect(),
    );
    let refresh_all = {
        let rows = rows.clone();
        move || rows.iter().for_each(AssocRow::refresh)
    };

    // まとめて操作するボタンと、OS の設定画面への入口
    let buttons = BoxSizer::builder(Orientation::Horizontal).build();
    let register_all = Button::builder(panel)
        .with_label(&tr("settings.assoc.register_all"))
        .build();
    let unregister_all = Button::builder(panel)
        .with_label(&tr("settings.assoc.unregister_all"))
        .build();
    buttons.add(&register_all, 0, SizerFlag::Right, px(GAP));
    buttons.add(&unregister_all, 0, SizerFlag::Right, px(GAP));
    register_all.on_click({
        let (rows, refresh_all, dialog) = (rows.clone(), refresh_all.clone(), dialog.clone());
        move |_| {
            let labels = assoc_labels();
            let result = rows.iter().try_for_each(|row| {
                let icon = row.chosen.borrow().clone();
                assoc::register(row.kind, &icon, &labels)
            });
            refresh_all();
            assoc_report(&dialog, result);
        }
    });
    unregister_all.on_click({
        let (rows, refresh_all, dialog) = (rows.clone(), refresh_all.clone(), dialog.clone());
        move |_| {
            // 解除できるもの (自分で登録した分) だけを解除する
            let result = rows
                .iter()
                .filter(|row| assoc::status(row.kind).removable)
                .try_for_each(|row| assoc::unregister(row.kind));
            refresh_all();
            assoc_report(&dialog, result);
        }
    });
    if caps.system_settings {
        let open_settings = Button::builder(panel)
            .with_label(&tr("settings.assoc.open_settings"))
            .build();
        buttons.add_stretch_spacer(1);
        buttons.add(&open_settings, 0, SizerFlag::AlignLeft, 0);
        open_settings.on_click(|_| assoc::open_system_settings());
    }
    sizer.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));

    // OS ごとの注意書き。関連付けを変えられないとき (macOS で .app として動いていない) はその理由
    let note = StaticText::builder(panel)
        .with_label(&tr(caps.unavailable.unwrap_or(caps.note)))
        .build();
    sizer.add(
        &note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(GAP),
    );
    panel.set_sizer(sizer, true);
    // 折り返しはサイザーに入れてから (page_preview と同じ)
    intro.wrap(px(520));
    note.wrap(px(520));

    if caps.unavailable.is_some() {
        register_all.enable(false);
        unregister_all.enable(false);
    }
    // 各行の幅が決まってから文字を入れる (幅に合わせてパスを省略するため)
    panel.layout();
    set_path_label(&target_label, &target, str::to_string);
    refresh_all();
    // 他のアプリや Windows の設定画面で変えた結果を、ダイアログに戻ってきたときに映す
    dialog.on_activate({
        let refresh_all = refresh_all.clone();
        move |event| {
            if let WindowEventData::Activate(activate) = &event {
                if activate.is_active() {
                    refresh_all();
                }
            }
        }
    });
}

/// パスの入った文を `label` に入れる。幅に収まらなければパスの途中を省略する (全文はツールチップ)。
/// wxStaticText は収まらない文を空白のところで折り返すので、そのままだと 1 行ぶんの高さの中に
/// パスが入らず見えなくなる。`path` が空なら `make` の結果をそのまま入れる
fn set_path_label(label: &StaticText, path: &str, make: impl Fn(&str) -> String) {
    let full = make(path);
    label.set_tooltip(&full);
    let width = label.get_size().width;
    let mut keep = path.chars().count();
    let mut text = full;
    while width > 0 && keep > 8 && label.get_text_extent(&text).width > width {
        keep -= (keep / 8).max(1);
        text = make(&shorten_middle(path, keep));
    }
    label.set_label(&text);
}

/// 文字列の途中を「…」にして `max` 文字に収める (パスは先頭のドライブと末尾のファイル名が手がかりになる)
fn shorten_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 2;
    let tail = &chars[chars.len() - (keep - head)..];
    chars[..head].iter().chain(&['…']).chain(tail).collect()
}

/// 関連付けのページに 1 形式ぶんの行を足す:
/// アイコンの見本 | 形式の名前・今の既定のアプリ・Typball の登録状況 | アイコンの選択・関連付ける・解除
fn assoc_row(
    panel: &Panel,
    sizer: &BoxSizer,
    dialog: &Dialog,
    kind: assoc::Kind,
    ft: FileType,
    caps: assoc::Capabilities,
) -> AssocRow {
    let px = |v: i32| panel.from_dip_int(v);
    let line = BoxSizer::builder(Orientation::Horizontal).build();

    // アイコンの見本 (中身は refresh で入れる)
    let preview = caps.icons.then(|| {
        let size = px(ASSOC_ICON_SIZE);
        let empty = vec![0u8; (size * size * 4) as usize];
        let bitmap = Bitmap::from_rgba(&empty, size as u32, size as u32);
        let preview = StaticBitmap::builder(panel)
            .with_bitmap(bitmap)
            .with_size(Size::new(size, size))
            .build();
        line.add(&preview, 0, SizerFlag::Right, px(GAP));
        preview
    });

    // 名前と状態。幅は固定の下限にして、長いパスが入っても右のボタンを押し出さないようにする
    let texts = BoxSizer::builder(Orientation::Vertical).build();
    let title = StaticText::builder(panel)
        .with_label(&tr(ft.name_key()))
        .build();
    if let Some(mut font) = title.get_font() {
        font.make_bold();
        title.set_font(&font);
    }
    texts.add(&title, 0, SizerFlag::Expand, 0);
    let status_line = || {
        let text = StaticText::builder(panel).build();
        text.set_min_size(Size::new(px(120), -1));
        texts.add(&text, 0, SizerFlag::Expand | SizerFlag::Top, px(2));
        text
    };
    let default_text = status_line();
    let registered_text = status_line();
    line.add_sizer(&texts, 1, SizerFlag::Expand | SizerFlag::Right, px(GAP));

    // 操作: アイコンの選択 (選べる OS だけ) の下に「関連付ける」「解除」
    let controls = BoxSizer::builder(Orientation::Vertical).build();
    let choice = caps.icons.then(|| {
        let choice = choice_with(
            panel,
            vec![
                tr("settings.assoc.icon_document"),
                tr("settings.assoc.icon_app"),
                tr("settings.assoc.icon_custom"),
            ],
        );
        let icon_line = BoxSizer::builder(Orientation::Horizontal).build();
        icon_line.add(
            &label(panel, "settings.assoc.icon"),
            0,
            LABEL_FLAGS | SizerFlag::Right,
            px(GAP),
        );
        icon_line.add(&choice, 1, LABEL_FLAGS, 0);
        controls.add_sizer(&icon_line, 0, SizerFlag::Expand | SizerFlag::Bottom, px(4));
        choice
    });
    let register = Button::builder(panel)
        .with_label(&tr("settings.assoc.register"))
        .build();
    let unregister = Button::builder(panel)
        .with_label(&tr("settings.assoc.unregister"))
        .build();
    let actions = BoxSizer::builder(Orientation::Horizontal).build();
    actions.add(&register, 1, SizerFlag::Right, px(4));
    actions.add(&unregister, 1, SizerFlag::AlignLeft, 0);
    controls.add_sizer(&actions, 0, SizerFlag::Expand, 0);
    line.add_sizer(&controls, 0, SizerFlag::AlignLeft, 0);
    sizer.add_sizer(
        &line,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(GAP),
    );

    let row = AssocRow {
        kind,
        panel: *panel,
        default_text,
        registered_text,
        unregister,
        icon: preview.zip(choice),
        chosen: Rc::new(RefCell::new(assoc::Icon::Document)),
    };
    if caps.unavailable.is_some() {
        register.enable(false);
    }

    register.on_click({
        let (row, dialog) = (row.clone(), dialog.clone());
        move |_| {
            let icon = row.chosen.borrow().clone();
            let result = assoc::register(row.kind, &icon, &assoc_labels());
            row.refresh();
            assoc_report(&dialog, result);
        }
    });
    unregister.on_click({
        let (row, dialog) = (row.clone(), dialog.clone());
        move |_| {
            let result = assoc::unregister(row.kind);
            row.refresh();
            assoc_report(&dialog, result);
        }
    });
    if let Some(choice) = choice {
        choice.on_selection_changed({
            let (row, dialog) = (row.clone(), dialog.clone());
            move |_| {
                let icon = match choice.get_selection() {
                    Some(0) => Some(assoc::Icon::Document),
                    Some(1) => Some(assoc::Icon::App),
                    // 「ファイルを指定」: 選ばずに閉じたら元の選択に戻す
                    _ => pick_icon_file(&dialog),
                };
                let Some(icon) = icon else {
                    row.show_icon();
                    return;
                };
                *row.chosen.borrow_mut() = icon.clone();
                // 登録済みならすぐ OS に反映する (未登録なら、次に関連付けるときに使う)
                let result = assoc::set_icon(row.kind, &icon);
                row.refresh();
                assoc_report(&dialog, result);
            }
        });
    }
    row
}

/// アイコンにするファイルを選ばせる
fn pick_icon_file(dialog: &Dialog) -> Option<assoc::Icon> {
    let picker = FileDialog::builder(dialog)
        .with_message(&tr("settings.assoc.icon_custom_title"))
        .with_wildcard(assoc::icon_wildcard())
        .with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
        .build();
    if picker.show_modal() != ID_OK {
        return None;
    }
    let path = picker.get_path()?;
    Some(assoc::custom_icon(std::path::Path::new(&path)))
}

fn page_update(panel: &Panel, current: &Settings, readers: &mut Readers, dialog: &Dialog) {
    let px = |v: i32| panel.from_dip_int(v);
    let check = CheckBox::builder(panel)
        .with_label(&tr("settings.update.check_on_startup"))
        .build();
    check.set_value(current.update.check_on_startup);

    let grid = form_grid(panel);
    let channel = choice_with(
        panel,
        Channel::ALL.iter().map(|c| tr(channel_key(*c))).collect(),
    );
    let selected = Channel::ALL
        .iter()
        .position(|c| *c == current.update.channel)
        .unwrap_or(0);
    channel.set_selection(selected as u32);
    grid.add(&label(panel, "settings.update.channel"), 0, LABEL_FLAGS, 0);
    grid.add(&channel, 0, SizerFlag::AlignLeft, 0);
    let interval = SpinCtrl::builder(panel)
        .with_range(1, 24 * 30)
        .with_initial_value(current.update.interval_hours.clamp(1, 24 * 30) as i32)
        .build();
    grid.add(&label(panel, "settings.update.interval"), 0, LABEL_FLAGS, 0);
    grid.add(&interval, 0, SizerFlag::AlignLeft, 0);

    let version = crate::update::version_label();
    let note = StaticText::builder(panel)
        .with_label(&tr_args(
            "settings.update.note",
            &[("url", crate::update::RELEASES_URL), ("version", &version)],
        ))
        .build();
    note.wrap(px(520));

    // 手動の確認。ダイアログを OK と同じく閉じて (チャネルの変更なども反映して) から確認する。
    // 開いたまま確認すると、更新の適用 (全ウィンドウを閉じて再起動) が設定ダイアログと衝突する
    let check_now = Button::builder(panel)
        .with_label(&tr("settings.update.check_now"))
        .build();
    check_now.on_click({
        let dialog = dialog.clone();
        move |_| dialog.end_modal(ID_CHECK_UPDATE)
    });

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add(&check, 0, SizerFlag::All, px(GAP));
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    sizer.add(
        &note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(GAP),
    );
    sizer.add(&check_now, 0, SizerFlag::All, px(GAP));
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        s.update.check_on_startup = check.get_value();
        s.update.channel = channel
            .get_selection()
            .and_then(|i| Channel::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.update.interval_hours = interval.value().max(1) as u32;
    }));
}

fn channel_key(value: Channel) -> &'static str {
    match value {
        Channel::Stable => "settings.update.channel_stable",
        Channel::Nightly => "settings.update.channel_nightly",
    }
}

fn markdown_mode_key(value: MarkdownMode) -> &'static str {
    match value {
        MarkdownMode::Auto => "settings.preview.markdown_mode_auto",
        MarkdownMode::Markdown => "settings.preview.markdown_mode_markdown",
        MarkdownMode::Site => "settings.preview.markdown_mode_site",
    }
}

fn external_change_key(value: ExternalChange) -> &'static str {
    match value {
        ExternalChange::Ask => "settings.files.external_change_ask",
        ExternalChange::Reload => "settings.files.external_change_reload",
        ExternalChange::Ignore => "settings.files.external_change_ignore",
    }
}

/// 保存のしかたの表示名のキー
fn save_method_key(value: SaveMethod) -> &'static str {
    match value {
        SaveMethod::Atomic => "settings.files.save_method_atomic",
        SaveMethod::Direct => "settings.files.save_method_direct",
    }
}

/// 行番号の位置の表示名のキー
fn line_numbers_key(value: LineNumbers) -> &'static str {
    match value {
        LineNumbers::None => "settings.editor.line_numbers_none",
        LineNumbers::Left => "settings.editor.line_numbers_left",
        LineNumbers::Right => "settings.editor.line_numbers_right",
        LineNumbers::Both => "settings.editor.line_numbers_both",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorten_middle_keeps_both_ends() {
        assert_eq!(shorten_middle("short", 10), "short");
        assert_eq!(shorten_middle("abcdefghij", 10), "abcdefghij");
        assert_eq!(shorten_middle("abcdefghijk", 10), "abcd…ghijk");
        let path = r"C:\Users\someone\AppData\Local\Programs\Typball\Typball.exe";
        let short = shorten_middle(path, 30);
        assert_eq!(short.chars().count(), 30);
        assert!(short.starts_with(r"C:\Users\some"));
        assert!(short.ends_with(r"all\Typball.exe"));
        // 日本語のパスも文字単位で切る
        assert_eq!(shorten_middle("あいうえおかきくけこ", 5), "あい…けこ");
    }
}

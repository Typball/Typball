//! 書き出し画面 (アクション → 書き出し...)
//!
//! ```text
//! 形式: [PDF ▾]
//! ┌ 書き出し先 ──────────────────────────┐
//! │ [C:\docs\report.pdf               ] [参照...] │
//! └──────────────────────────────────────┘
//! ┌ PDF の設定 ───────────────────────────┐   (形式ごとに中身が変わる)
//! │ PDF のバージョン: [PDF 1.7 ▾]                │
//! │ PDF/A:            [なし ▾]                   │
//! │ [x] タグ付き PDF   [ ] PDF/UA-1              │
//! └──────────────────────────────────────┘
//!                                 [保存] [キャンセル]
//! ```
//!
//! - Typst: PDF / PNG (全ページを 1 枚に・ページごと) / SVG (同)。コンパイルし直して書き出す
//! - Markdown: PDF (WebView2 の印刷で作る。見えない WebView に書き出し用のページを読み込んで印刷する
//!   ので、終わるまでダイアログに「書き出しています」と出して待つ) / HTML (CSS を埋め込んだ 1 ファイル)
//! - 最後に選んだ形式と設定は `[export]` に保存する

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use wxdragon::prelude::*;

use crate::filetype::FileType;
use crate::i18n::{tr, tr_args};
use crate::settings::{self, Export};
use crate::webpreview::{self, PdfExport};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    /// Typst → PDF
    Pdf,
    /// Typst → 全ページを縦に並べた 1 枚の PNG
    Png,
    /// Typst → ページごとの PNG
    PngPages,
    Svg,
    SvgPages,
    /// Markdown → PDF (WebView2)
    MdPdf,
    /// Markdown → HTML
    Html,
}

impl Format {
    fn for_filetype(filetype: FileType) -> &'static [Format] {
        use Format::*;
        match filetype {
            FileType::Typ => &[Pdf, Png, PngPages, Svg, SvgPages],
            FileType::Md => &[MdPdf, Html],
            FileType::Txt => &[],
        }
    }

    /// 設定ファイルの値
    fn key(self) -> &'static str {
        match self {
            Format::Pdf | Format::MdPdf => "pdf",
            Format::Png => "png",
            Format::PngPages => "png_pages",
            Format::Svg => "svg",
            Format::SvgPages => "svg_pages",
            Format::Html => "html",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Format::Pdf | Format::MdPdf => "export.format_pdf",
            Format::Png => "export.format_png",
            Format::PngPages => "export.format_png_pages",
            Format::Svg => "export.format_svg",
            Format::SvgPages => "export.format_svg_pages",
            Format::Html => "export.format_html",
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Format::Pdf | Format::MdPdf => "pdf",
            Format::Png | Format::PngPages => "png",
            Format::Svg | Format::SvgPages => "svg",
            Format::Html => "html",
        }
    }

    fn per_page(self) -> bool {
        matches!(self, Format::PngPages | Format::SvgPages)
    }

    fn group(self) -> Group {
        match self {
            Format::Pdf => Group::Pdf,
            Format::Png | Format::PngPages => Group::Png,
            Format::Svg | Format::SvgPages => Group::None,
            Format::MdPdf => Group::MdPdf,
            Format::Html => Group::Html,
        }
    }
}

/// 設定の段 (形式によって出し分ける)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Group {
    Pdf,
    Png,
    MdPdf,
    Html,
    None,
}

const DPIS: [u32; 7] = [72, 96, 144, 150, 200, 300, 600];
/// 全ページを 1 枚にまとめるときのページ間のすき間 (pt)
const MERGE_GAP_PT: f64 = 10.0;

/// 書き出す文書
pub struct Source<'a> {
    pub filetype: FileType,
    pub text: &'a str,
    /// 保存済みならそのパス (画像の相対パスの基準・既定の書き出し先)
    pub path: Option<&'a Path>,
    /// タブの名前 (無題のときの既定のファイル名)
    pub title: &'a str,
}

/// 書き出し画面を開く。書き出したらステータスバーに出す文を返す
pub fn show(parent: &Frame, source: &Source) -> Option<String> {
    let formats = Format::for_filetype(source.filetype);
    if formats.is_empty() {
        return None;
    }
    let saved = settings::get().export;
    let dialog = Dialog::builder(parent, &tr("export.title"))
        .with_style(DialogStyle::DefaultDialogStyle)
        .build();
    let px = |v: i32| dialog.from_dip_int(v);

    // 形式
    let format_choice = Choice::builder(&dialog).build();
    format_choice.freeze();
    for f in formats {
        format_choice.append(&tr(f.label_key()));
    }
    format_choice.thaw();
    let wanted = if source.filetype == FileType::Md {
        &saved.markdown_format
    } else {
        &saved.typst_format
    };
    let initial = formats.iter().position(|f| f.key() == wanted).unwrap_or(0);
    format_choice.set_selection(initial as u32);

    // 書き出し先
    let dest_box = StaticBoxSizerBuilder::new_with_label(
        Orientation::Vertical,
        &dialog,
        &tr("export.destination"),
    )
    .build();
    let path_ctrl = TextCtrl::builder(&dialog).build();
    path_ctrl.set_min_size(Size::new(px(380), -1));
    let browse = Button::builder(&dialog)
        .with_label(&tr("export.browse"))
        .build();
    let per_page_note = StaticText::builder(&dialog).with_label("").build();
    let path_row = BoxSizer::builder(Orientation::Horizontal).build();
    path_row.add(&path_ctrl, 1, SizerFlag::AlignCenterVertical, 0);
    path_row.add(
        &browse,
        0,
        SizerFlag::AlignCenterVertical | SizerFlag::Left,
        px(6),
    );
    dest_box.add_sizer(&path_row, 0, SizerFlag::Expand | SizerFlag::All, px(6));
    dest_box.add(
        &per_page_note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(6),
    );

    // 形式ごとの設定。段ごとに Panel に入れて出し分ける (wxDragon のサイザーには表示の切り替えが無い)
    let label = |parent: &Panel, text: &str| StaticText::builder(parent).with_label(text).build();
    let group_panel = |title: &str| {
        let panel = Panel::builder(&dialog).build();
        let sizer =
            StaticBoxSizerBuilder::new_with_label(Orientation::Vertical, &panel, title).build();
        (panel, sizer)
    };
    let grid = || {
        let g = FlexGridSizer::builder(0, 2)
            .with_vgap(px(6))
            .with_hgap(px(8))
            .build();
        g.add_growable_col(1, 1);
        g
    };
    let center = SizerFlag::AlignCenterVertical;

    // PDF (Typst)
    let (pdf_panel, pdf_box) = group_panel(&tr("export.pdf_settings"));
    let pdf_version = Choice::builder(&pdf_panel).build();
    for v in typball_core::PDF_VERSIONS {
        pdf_version.append(&format!("PDF {v}"));
    }
    let pdf_a = Choice::builder(&pdf_panel).build();
    for a in typball_core::PDF_A_STANDARDS {
        pdf_a.append(&if a.is_empty() {
            tr("export.none")
        } else {
            format!("PDF/A-{}", &a[2..])
        });
    }
    let pdf_tagged = CheckBox::builder(&pdf_panel)
        .with_label(&tr("export.pdf_tagged"))
        .build();
    let pdf_ua = CheckBox::builder(&pdf_panel)
        .with_label(&tr("export.pdf_ua"))
        .build();
    let g = grid();
    let l1 = label(&pdf_panel, &tr("export.pdf_version"));
    let l2 = label(&pdf_panel, &tr("export.pdf_a"));
    g.add(&l1, 0, center, 0);
    g.add(&pdf_version, 1, SizerFlag::Expand, 0);
    g.add(&l2, 0, center, 0);
    g.add(&pdf_a, 1, SizerFlag::Expand, 0);
    pdf_box.add_sizer(&g, 0, SizerFlag::Expand | SizerFlag::All, px(6));
    let checks = BoxSizer::builder(Orientation::Horizontal).build();
    checks.add(&pdf_tagged, 0, center, 0);
    checks.add(&pdf_ua, 0, center | SizerFlag::Left, px(16));
    pdf_box.add_sizer(
        &checks,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(6),
    );
    pdf_panel.set_sizer(pdf_box, true);

    // PNG
    let (png_panel, png_box) = group_panel(&tr("export.png_settings"));
    let png_dpi = Choice::builder(&png_panel).build();
    for d in DPIS {
        png_dpi.append(&format!("{d} dpi"));
    }
    let l3 = label(&png_panel, &tr("export.png_dpi"));
    let g = grid();
    g.add(&l3, 0, center, 0);
    g.add(&png_dpi, 1, SizerFlag::Expand, 0);
    png_box.add_sizer(&g, 0, SizerFlag::Expand | SizerFlag::All, px(6));
    png_panel.set_sizer(png_box, true);

    // Markdown → PDF
    let (md_panel, md_box) = group_panel(&tr("export.pdf_settings"));
    let paper = Choice::builder(&md_panel).build();
    for p in webpreview::PAPERS {
        paper.append(p);
    }
    let orientation = Choice::builder(&md_panel).build();
    orientation.append(&tr("export.portrait"));
    orientation.append(&tr("export.landscape"));
    let l4 = label(&md_panel, &tr("export.paper"));
    let l5 = label(&md_panel, &tr("export.orientation"));
    let md_note = StaticText::builder(&md_panel)
        .with_label(&tr("export.md_pdf_note"))
        .build();
    let g = grid();
    g.add(&l4, 0, center, 0);
    g.add(&paper, 1, SizerFlag::Expand, 0);
    g.add(&l5, 0, center, 0);
    g.add(&orientation, 1, SizerFlag::Expand, 0);
    md_box.add_sizer(&g, 0, SizerFlag::Expand | SizerFlag::All, px(6));
    md_box.add(
        &md_note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(6),
    );
    md_panel.set_sizer(md_box, true);

    // HTML
    let (html_panel, html_box) = group_panel(&tr("export.html_settings"));
    let html_embed = CheckBox::builder(&html_panel)
        .with_label(&tr("export.html_embed"))
        .build();
    html_box.add(&html_embed, 0, SizerFlag::All, px(6));
    html_panel.set_sizer(html_box, true);

    // 設定の無い形式 (SVG)
    let (none_panel, none_box) = group_panel(&tr("export.settings"));
    let none_text = StaticText::builder(&none_panel)
        .with_label(&tr("export.no_settings"))
        .build();
    none_box.add(&none_text, 0, SizerFlag::All, px(6));
    none_panel.set_sizer(none_box, true);

    // 既定値
    let version_index = typball_core::PDF_VERSIONS
        .iter()
        .position(|v| *v == saved.pdf_version)
        .unwrap_or(3);
    pdf_version.set_selection(version_index as u32);
    let a_index = typball_core::PDF_A_STANDARDS
        .iter()
        .position(|a| *a == saved.pdf_a)
        .unwrap_or(0);
    pdf_a.set_selection(a_index as u32);
    pdf_tagged.set_value(saved.pdf_tagged || saved.pdf_ua);
    pdf_ua.set_value(saved.pdf_ua);
    pdf_tagged.enable(!saved.pdf_ua);
    let dpi_index = DPIS.iter().position(|d| *d == saved.png_dpi).unwrap_or(2);
    png_dpi.set_selection(dpi_index as u32);
    let paper_index = webpreview::PAPERS
        .iter()
        .position(|p| *p == saved.paper)
        .unwrap_or(0);
    paper.set_selection(paper_index as u32);
    orientation.set_selection(u32::from(saved.landscape));
    html_embed.set_value(saved.html_embed_images);

    // PDF/A を選んだら、それが求めるバージョンに合わせる。PDF/UA はタグ付きが前提
    pdf_a.on_selection_changed(move |_| {
        let Some(a) = pdf_a
            .get_selection()
            .and_then(|i| typball_core::PDF_A_STANDARDS.get(i as usize))
        else {
            return;
        };
        if let Some(v) = typball_core::pdfa_version(a) {
            if let Some(i) = typball_core::PDF_VERSIONS.iter().position(|x| *x == v) {
                pdf_version.set_selection(i as u32);
            }
        }
    });
    pdf_ua.on_toggled(move |_| {
        let ua = pdf_ua.is_checked();
        if ua {
            pdf_tagged.set_value(true);
        }
        pdf_tagged.enable(!ua);
    });

    // ボタンと進み具合
    let progress = StaticText::builder(&dialog).with_label("").build();
    // ID_OK にしない: wx の既定の処理でクリックした瞬間にダイアログが閉じてしまい、Markdown の PDF
    // (非同期) が書き出される前に WebView ごと消える。閉じるのは書き出しが済んでから自分で行う
    let save = Button::builder(&dialog)
        .with_label(&tr("export.save"))
        .build();
    let cancel = Button::builder(&dialog)
        .with_id(ID_CANCEL)
        .with_label(&tr("export.cancel"))
        .build();
    save.set_default();
    let buttons = BoxSizer::builder(Orientation::Horizontal).build();
    buttons.add(&progress, 1, center, 0);
    buttons.add(&save, 0, center | SizerFlag::Left, px(8));
    buttons.add(&cancel, 0, center | SizerFlag::Left, px(8));

    let format_row = BoxSizer::builder(Orientation::Horizontal).build();
    let format_label = StaticText::builder(&dialog)
        .with_label(&tr("export.format"))
        .build();
    format_row.add(&format_label, 0, center, 0);
    format_row.add(&format_choice, 0, center | SizerFlag::Left, px(8));

    let root = BoxSizer::builder(Orientation::Vertical).build();
    let side = SizerFlag::Left | SizerFlag::Right;
    root.add_sizer(&format_row, 0, side | SizerFlag::Top, px(12));
    root.add_sizer(
        &dest_box,
        0,
        SizerFlag::Expand | side | SizerFlag::Top,
        px(12),
    );
    let groups = [
        (Group::Pdf, pdf_panel),
        (Group::Png, png_panel),
        (Group::MdPdf, md_panel),
        (Group::Html, html_panel),
        (Group::None, none_panel),
    ];
    for (_, panel) in &groups {
        root.add(panel, 0, SizerFlag::Expand | side | SizerFlag::Top, px(10));
    }
    root.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, px(12));
    dialog.set_sizer(root, true);

    // 既定の書き出し先: 文書のフォルダ (無題ならドキュメント)、名前は文書名
    let default_dir = source
        .path
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = Path::new(source.title)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "document".to_string());
    path_ctrl.set_value(
        &default_dir
            .join(format!("{stem}.{}", formats[initial].ext()))
            .display()
            .to_string(),
    );

    // 形式を変えたら: 設定の段を出し分け、拡張子を合わせる
    let current_format = move || {
        format_choice
            .get_selection()
            .and_then(|i| formats.get(i as usize))
            .copied()
            .unwrap_or(formats[0])
    };
    let apply_format = {
        move |change_ext: bool| {
            let format = current_format();
            let group = format.group();
            // 形式で使えない段 (Typst の文書に Markdown の段など) は最初から出さない
            for (g, panel) in &groups {
                panel.show(*g == group);
            }
            if change_ext {
                let text = path_ctrl.get_value();
                let path = PathBuf::from(text.trim());
                if !text.trim().is_empty() {
                    path_ctrl.set_value(&path.with_extension(format.ext()).display().to_string());
                }
            }
            per_page_note.set_label(&if format.per_page() {
                tr_args("export.per_page_note", &[("ext", format.ext())])
            } else {
                String::new()
            });
            per_page_note.show(format.per_page());
            dialog.layout();
            dialog.fit();
        }
    };
    let apply_format = Rc::new(apply_format);
    format_choice.on_selection_changed({
        let apply_format = apply_format.clone();
        move |_| apply_format(true)
    });
    apply_format(false);

    // 保存できるのは書き出し先が入っているときだけ
    path_ctrl.on_text_changed(move |_| save.enable(!path_ctrl.get_value().trim().is_empty()));

    // 参照... で選んだファイルは OS のダイアログで上書きを確認済み
    let confirmed: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    browse.on_click({
        let confirmed = confirmed.clone();
        let default_dir = default_dir.clone();
        move |_| {
            let format = current_format();
            let current = PathBuf::from(path_ctrl.get_value().trim());
            let dir = current
                .parent()
                .filter(|d| d.is_dir())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| default_dir.clone());
            let file = current
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut style = FileDialogStyle::Save;
            if !format.per_page() {
                style = style | FileDialogStyle::OverwritePrompt;
            }
            let ext = format.ext();
            let picker = FileDialog::builder(&dialog)
                .with_message(&tr("export.title"))
                .with_default_dir(&dir.display().to_string())
                .with_default_file(&file)
                .with_wildcard(&format!("*.{ext}|*.{ext}"))
                .with_style(style)
                .build();
            if picker.show_modal() == ID_OK {
                if let Some(path) = picker.get_path() {
                    let path = PathBuf::from(path);
                    let path = if path.extension().is_none() {
                        path.with_extension(ext)
                    } else {
                        path
                    };
                    path_ctrl.set_value(&path.display().to_string());
                    *confirmed.borrow_mut() = (!format.per_page()).then_some(path);
                }
            }
        }
    });

    // 保存
    let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let pdf_job: Rc<RefCell<Option<Rc<PdfExport>>>> = Rc::new(RefCell::new(None));
    let text = source.text.to_string();
    let doc_path = source.path.map(Path::to_path_buf);
    let filetype = source.filetype;
    save.on_click({
        let result = result.clone();
        let confirmed = confirmed.clone();
        let pdf_job = pdf_job.clone();
        move |_| {
            let format = current_format();
            let mut out = PathBuf::from(path_ctrl.get_value().trim());
            if out.as_os_str().is_empty() {
                return;
            }
            if out.is_relative() {
                out = default_dir.join(out);
            }
            if out.extension().is_none() {
                out.set_extension(format.ext());
            }
            if !out.parent().map(Path::is_dir).unwrap_or(false) {
                error(
                    &dialog,
                    &tr_args(
                        "export.no_folder",
                        &[("path", &out.parent().unwrap_or(&out).display().to_string())],
                    ),
                );
                return;
            }
            // 設定を覚える
            let export = Export {
                typst_format: if filetype == FileType::Typ {
                    format.key().to_string()
                } else {
                    saved.typst_format.clone()
                },
                markdown_format: if filetype == FileType::Md {
                    format.key().to_string()
                } else {
                    saved.markdown_format.clone()
                },
                pdf_version: selected(&pdf_version, typball_core::PDF_VERSIONS)
                    .unwrap_or("1.7")
                    .to_string(),
                pdf_a: selected(&pdf_a, typball_core::PDF_A_STANDARDS)
                    .unwrap_or("")
                    .to_string(),
                pdf_ua: pdf_ua.is_checked(),
                pdf_tagged: pdf_tagged.is_checked(),
                png_dpi: png_dpi
                    .get_selection()
                    .and_then(|i| DPIS.get(i as usize))
                    .copied()
                    .unwrap_or(144),
                html_embed_images: html_embed.is_checked(),
                paper: selected(&paper, &webpreview::PAPERS)
                    .unwrap_or("A4")
                    .to_string(),
                landscape: orientation.get_selection() == Some(1),
            };
            let mut s = settings::get();
            if s.export != export {
                s.export = export.clone();
                settings::set(s.clone());
                let _ = s.save();
            }

            let already_confirmed = confirmed.borrow().as_deref() == Some(out.as_path());
            match format {
                Format::MdPdf => {
                    if !already_confirmed
                        && out.exists()
                        && !confirm_overwrite(&dialog, &[out.clone()])
                    {
                        return;
                    }
                    // 見えない WebView で印刷する。終わるまで操作を止めて待つ
                    let set_busy = move |busy: bool| {
                        for w in [&format_choice as &dyn WxWidget, &path_ctrl, &browse, &save] {
                            w.enable(!busy);
                        }
                        progress.set_label(&if busy {
                            tr("export.working")
                        } else {
                            String::new()
                        });
                    };
                    set_busy(true);
                    let done = {
                        let result = result.clone();
                        let out = out.clone();
                        move |r: Result<(), String>| {
                            match r {
                                Ok(()) => {
                                    *result.borrow_mut() = Some(tr_args(
                                        "export.done",
                                        &[("path", &out.display().to_string())],
                                    ));
                                    dialog.end_modal(ID_OK);
                                }
                                Err(e) => {
                                    set_busy(false);
                                    error(&dialog, &tr_args("export.error", &[("error", &e)]));
                                }
                            }
                            // 完了の通知の中では WebView を消さない (ダイアログと一緒に消える)
                        }
                    };
                    match PdfExport::start(
                        &dialog,
                        &text,
                        doc_path.as_deref(),
                        &out,
                        paper.get_selection().unwrap_or(0) as usize,
                        orientation.get_selection() == Some(1),
                        done,
                    ) {
                        Ok(job) => *pdf_job.borrow_mut() = Some(job),
                        Err(e) => {
                            set_busy(false);
                            error(&dialog, &tr_args("export.error", &[("error", &e)]));
                        }
                    }
                }
                Format::Html => {
                    if !already_confirmed
                        && out.exists()
                        && !confirm_overwrite(&dialog, &[out.clone()])
                    {
                        return;
                    }
                    let html = webpreview::export_html(
                        &text,
                        doc_path.as_deref(),
                        &out,
                        html_embed.is_checked(),
                    );
                    match std::fs::write(&out, html) {
                        Ok(()) => {
                            *result.borrow_mut() = Some(tr_args(
                                "export.done",
                                &[("path", &out.display().to_string())],
                            ));
                            dialog.end_modal(ID_OK);
                        }
                        Err(e) => error(
                            &dialog,
                            &tr_args("export.error", &[("error", &e.to_string())]),
                        ),
                    }
                }
                _ => {
                    let wait = BusyCursor::new(None);
                    let outcome = export_typst(&text, format, &out, &export, |files| {
                        let unconfirmed: Vec<PathBuf> = files
                            .iter()
                            .filter(|f| f.exists() && !(already_confirmed && **f == out))
                            .cloned()
                            .collect();
                        unconfirmed.is_empty() || confirm_overwrite(&dialog, &unconfirmed)
                    });
                    drop(wait);
                    match outcome {
                        Ok(Some(files)) => {
                            let first = files.first().cloned().unwrap_or_else(|| out.clone());
                            *result.borrow_mut() = Some(if files.len() > 1 {
                                tr_args(
                                    "export.done_pages",
                                    &[
                                        ("count", &files.len().to_string()),
                                        ("path", &first.display().to_string()),
                                    ],
                                )
                            } else {
                                tr_args("export.done", &[("path", &first.display().to_string())])
                            });
                            dialog.end_modal(ID_OK);
                        }
                        Ok(None) => {}
                        Err(e) => error(&dialog, &tr_args("export.error", &[("error", &e)])),
                    }
                }
            }
        }
    });

    dialog.fit();
    dialog.centre();
    dialog.layout();
    let code = dialog.show_modal();
    // 書き出し中の WebView はダイアログの子なので、ここで一緒に消える
    pdf_job.borrow_mut().take();
    dialog.destroy();
    if code == ID_OK {
        result.borrow_mut().take()
    } else {
        None
    }
}

fn selected<'a>(choice: &Choice, values: &[&'a str]) -> Option<&'a str> {
    choice
        .get_selection()
        .and_then(|i| values.get(i as usize))
        .copied()
}

/// Typst を書き出す。`confirm` は書き出すファイルの一覧を受け取り、上書きしてよければ true。
/// 書いたファイルの一覧を返す (確認で取りやめたら None)
fn export_typst(
    text: &str,
    format: Format,
    out: &Path,
    export: &Export,
    confirm: impl Fn(&[PathBuf]) -> bool,
) -> Result<Option<Vec<PathBuf>>, String> {
    let (doc, diagnostics) = typball_core::compile_doc(text);
    let Some(doc) = doc else {
        return Err(diagnostics.join("\n"));
    };
    let dpi = export.png_dpi as f32;
    let outputs: Vec<(PathBuf, Vec<u8>)> = match format {
        Format::Pdf => {
            let pdf = doc.pdf(&typball_core::PdfSettings {
                version: export.pdf_version.clone(),
                pdfa: export.pdf_a.clone(),
                pdfua: export.pdf_ua,
                tagged: export.pdf_tagged,
            })?;
            vec![(out.to_path_buf(), pdf)]
        }
        Format::Png => vec![(out.to_path_buf(), doc.png_merged(dpi, MERGE_GAP_PT)?)],
        Format::Svg => vec![(out.to_path_buf(), doc.svg_merged(MERGE_GAP_PT).into_bytes())],
        Format::PngPages => {
            let pages = doc.png_pages(dpi)?;
            page_files(out, pages.len())
                .into_iter()
                .zip(pages)
                .collect()
        }
        Format::SvgPages => {
            let pages = doc.svg_pages();
            page_files(out, pages.len())
                .into_iter()
                .zip(pages.into_iter().map(String::into_bytes))
                .collect()
        }
        Format::MdPdf | Format::Html => return Err("not a Typst format".to_string()),
    };
    let files: Vec<PathBuf> = outputs.iter().map(|(p, _)| p.clone()).collect();
    if !confirm(&files) {
        return Ok(None);
    }
    for (path, data) in &outputs {
        std::fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(Some(files))
}

/// ページごとのファイル名: `report.png` → `report-1.png`, `report-2.png` … (ページ数の桁で 0 埋め)
fn page_files(out: &Path, count: usize) -> Vec<PathBuf> {
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = out
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let width = count.max(1).to_string().len();
    (1..=count)
        .map(|n| out.with_file_name(format!("{stem}-{n:0width$}.{ext}")))
        .collect()
}

fn confirm_overwrite(parent: &Dialog, files: &[PathBuf]) -> bool {
    let list = files
        .iter()
        .take(5)
        .map(|f| f.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let more = if files.len() > 5 {
        format!("\n… (+{})", files.len() - 5)
    } else {
        String::new()
    };
    MessageDialog::builder(
        parent,
        &tr_args("export.overwrite", &[("files", &format!("{list}{more}"))]),
        "Typball",
    )
    .with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
    .build()
    .show_modal()
        == ID_YES
}

fn error(parent: &Dialog, message: &str) {
    MessageDialog::builder(parent, message, "Typball")
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
        .build()
        .show_modal();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_files_are_numbered_with_padding() {
        let files = page_files(Path::new("C:/out/report.png"), 12);
        assert_eq!(files.len(), 12);
        assert_eq!(files[0], Path::new("C:/out/report-01.png"));
        assert_eq!(files[11], Path::new("C:/out/report-12.png"));
        assert_eq!(page_files(Path::new("a.svg"), 3)[2], Path::new("a-3.svg"));
    }

    #[test]
    fn formats_match_filetypes() {
        assert!(Format::for_filetype(FileType::Txt).is_empty());
        assert_eq!(Format::for_filetype(FileType::Md)[0], Format::MdPdf);
        for f in Format::for_filetype(FileType::Typ) {
            assert!(!matches!(f, Format::MdPdf | Format::Html));
        }
    }
}

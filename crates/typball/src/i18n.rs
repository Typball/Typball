//! 多言語対応 (UI 文字列の切り替え)
//!
//! 言語ごとの文字列は `locales/<code>.toml` に置き、ビルド時に実行ファイルへ埋め込む。
//! ネストしたテーブルは "menu.file" のようにドット区切りのキーに平坦化される。
//!
//! - [`set_language`] で現在の言語を切り替える ("system" なら OS の表示言語から決める)。
//! - [`tr`] / [`tr_args`] でキーから文字列を得る。見つからなければ英語 → キーそのものの順で
//!   フォールバックするので、翻訳漏れがあっても UI が空になることはない。
//!
//! 新しい言語を追加するには `locales/xx.toml` を作って [`LOCALES`] に 1 行足すだけでよい。

use std::collections::HashMap;
use std::sync::RwLock;

/// 埋め込む言語ファイル。先頭 (英語) がフォールバック元になる。
const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.toml")),
    ("ja", include_str!("../locales/ja.toml")),
];

/// "OS の言語に従う" を表す設定値
pub const SYSTEM: &str = "system";

struct Catalog {
    strings: HashMap<String, String>,
}

static CATALOG: RwLock<Option<Catalog>> = RwLock::new(None);

/// 利用できる言語の一覧: (言語コード, その言語での名前)
pub fn available() -> Vec<(&'static str, String)> {
    LOCALES
        .iter()
        .map(|(code, src)| {
            let name = parse(src)
                .remove("meta.native_name")
                .unwrap_or_else(|| code.to_string());
            (*code, name)
        })
        .collect()
}

/// 設定値 ("system" / "ja" / ...) を実際に使う言語コードに解決する
pub fn resolve(setting: &str) -> &'static str {
    let wanted = if setting.is_empty() || setting == SYSTEM {
        sys_locale::get_locale().unwrap_or_default()
    } else {
        setting.to_string()
    };
    // "ja-JP" / "ja_JP" / "ja" のいずれでも先頭の言語部分で照合する
    let primary = wanted
        .split(['-', '_', '.'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    LOCALES
        .iter()
        .map(|(code, _)| *code)
        .find(|code| *code == primary)
        .unwrap_or(LOCALES[0].0)
}

/// 現在の言語を切り替える。`setting` は設定ファイルの値そのまま ("system" 可)。
pub fn set_language(setting: &str) {
    let code = resolve(setting);
    let mut strings = parse(LOCALES[0].1);
    if code != LOCALES[0].0 {
        if let Some((_, src)) = LOCALES.iter().find(|(c, _)| *c == code) {
            strings.extend(parse(src));
        }
    }
    *CATALOG.write().unwrap() = Some(Catalog { strings });
}

/// キーに対応する文字列を返す
pub fn tr(key: &str) -> String {
    if let Some(cat) = CATALOG.read().unwrap().as_ref() {
        if let Some(s) = cat.strings.get(key) {
            return s.clone();
        }
    }
    key.to_string()
}

/// `{name}` 形式のプレースホルダを置き換えて返す
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = tr(key);
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

/// TOML を "a.b.c" = "値" の平坦なマップにする
fn parse(src: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    match src.parse::<toml::Table>() {
        Ok(table) => flatten("", &table, &mut out),
        Err(e) => eprintln!("typball: locale file is invalid: {e}"),
    }
    out
}

fn flatten(prefix: &str, table: &toml::Table, out: &mut HashMap<String, String>) {
    for (k, v) in table {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            toml::Value::Table(t) => flatten(&key, t, out),
            toml::Value::String(s) => {
                out.insert(key, s.clone());
            }
            other => {
                out.insert(key, other.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_has_the_english_keys() {
        let en = parse(LOCALES[0].1);
        for (code, src) in LOCALES {
            let map = parse(src);
            for key in en.keys() {
                assert!(map.contains_key(key), "{code}.toml lacks key {key}");
            }
            for key in map.keys() {
                assert!(en.contains_key(key), "{code}.toml has extra key {key}");
            }
        }
    }

    #[test]
    fn placeholders_are_substituted() {
        set_language("en");
        assert_eq!(
            tr_args("status.position", &[("line", "3"), ("col", "7")]),
            "Ln 3, Col 7"
        );
    }
}

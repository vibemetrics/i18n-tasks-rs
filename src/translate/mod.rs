//! `translate-missing`: the base locale's values for the keys a locale lacks,
//! translated and written. Accepted diff 34.
//!
//! ref: lib/i18n/tasks/command/commands/missing.rb:40-57
//! ref: lib/i18n/tasks/translators/base_translator.rb

pub mod language_names;
pub mod openai;

use crate::config::{Config, IgnoreType, Router};
use crate::data::load::{Leaf, Store, Value};
use crate::data::route::{self, ConservativeRouter, PatternRouter};
use crate::plural::depluralize_key;
use crate::report::interpolations::variables;
use crate::report::missing::locale_key_missing;
use crate::report::normalize::{self, NormalizeReport};
use crate::report::{KeyRow, render_table};
use crate::session::Session;
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, LazyLock};

pub const BATCH_SIZE: usize = 50;

#[allow(
    clippy::expect_used,
    reason = "a static pattern that fails to compile is a bug here, not a run-time condition"
)]
static INTERPOLATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"%\{[^}]+\}").expect("static pattern compiles"));

#[allow(
    clippy::expect_used,
    reason = "a static pattern that fails to compile is a bug here, not a run-time condition"
)]
static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"X__(\d+)").expect("static pattern compiles"));

#[derive(Debug, Clone)]
pub struct Pending {
    pub locale: String,
    pub leaves: Vec<Leaf>,
}

#[derive(Debug)]
pub struct Failure {
    pub keys: Vec<String>,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct Translated {
    pub leaves: Vec<Leaf>,
    pub failures: Vec<Failure>,
}

/// ref: missing_keys.rb#missing_diff_tree
pub fn pending(cfg: &Config, store: &Store, locales: &[String]) -> Vec<Pending> {
    let base = &store.base_locale;
    let Some(source) = store.tree(base) else {
        return Vec::new();
    };
    locales
        .iter()
        .filter(|l| *l != base)
        .map(|locale| {
            let ignore = cfg.ignore_patterns(IgnoreType::Missing, Some(locale));
            let leaves = source
                .sorted_keys()
                .into_iter()
                .filter(|leaf| {
                    let key = depluralize_key(&leaf.key, Some(source), Some(source));
                    locale_key_missing(store, &ignore, locale, &key)
                })
                .cloned()
                .collect();
            Pending {
                locale: locale.clone(),
                leaves,
            }
        })
        .filter(|p| !p.leaves.is_empty())
        .collect()
}

/// ref: base_translator.rb#translate_pairs
pub fn translate(
    leaves: &[Leaf],
    mut call: impl FnMut(&[String]) -> Result<Vec<String>, String>,
) -> Translated {
    let mut values: Vec<Option<Value>> = vec![None; leaves.len()];
    let mut failures = Vec::new();
    let mut batch: Vec<(usize, Vec<String>)> = Vec::new();
    let mut size = 0;
    let mut flush = |batch: &mut Vec<(usize, Vec<String>)>, values: &mut Vec<Option<Value>>| {
        if batch.is_empty() {
            return;
        }
        match run_batch(leaves, batch, &mut call) {
            Ok(translated) => {
                for ((i, _), value) in batch.iter().zip(translated) {
                    values[*i] = Some(value);
                }
            }
            Err(error) => failures.push(Failure {
                keys: batch.iter().map(|(i, _)| leaves[*i].key.clone()).collect(),
                error,
            }),
        }
        batch.clear();
    };
    for (i, leaf) in leaves.iter().enumerate() {
        let mut strings = Vec::new();
        dump(&leaf.value, &mut strings);
        if strings.is_empty() {
            values[i] = Some(leaf.value.clone());
            continue;
        }
        if size + strings.len() > BATCH_SIZE {
            flush(&mut batch, &mut values);
            size = 0;
        }
        size += strings.len();
        batch.push((i, strings));
    }
    flush(&mut batch, &mut values);
    Translated {
        leaves: leaves
            .iter()
            .zip(values)
            .filter_map(|(leaf, value)| {
                value.map(|value| Leaf {
                    value,
                    ..leaf.clone()
                })
            })
            .collect(),
        failures,
    }
}

fn run_batch(
    leaves: &[Leaf],
    batch: &[(usize, Vec<String>)],
    call: &mut impl FnMut(&[String]) -> Result<Vec<String>, String>,
) -> Result<Vec<Value>, String> {
    let sent: Vec<String> = batch.iter().flat_map(|(_, s)| s.iter().cloned()).collect();
    let received = call(&sent)?;
    if received.len() != sent.len() {
        return Err(format!(
            "sent {} strings, received {} translations",
            sent.len(),
            received.len()
        ));
    }
    let mut received = received.into_iter();
    batch
        .iter()
        .map(|(i, _)| implode(&leaves[*i].value, &mut received))
        .collect()
}

fn dump(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Str(s) if !s.is_empty() => out.push(protect(s)),
        Value::Seq(items) => items.iter().for_each(|v| dump(v, out)),
        Value::Map(entries) => entries.iter().for_each(|(_, v)| dump(v, out)),
        _ => {}
    }
}

fn implode(value: &Value, translated: &mut impl Iterator<Item = String>) -> Result<Value, String> {
    Ok(match value {
        Value::Str(s) if !s.is_empty() => {
            let text = translated
                .next()
                .ok_or_else(|| "fewer translations than strings".to_string())?;
            Value::Str(restore(s, &text)?)
        }
        Value::Seq(items) => Value::Seq(
            items
                .iter()
                .map(|v| implode(v, translated))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(entries) => Value::Map(
            entries
                .iter()
                .map(|(k, v)| Ok((k.clone(), implode(v, translated)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

fn protect(value: &str) -> String {
    let mut i = 0;
    INTERPOLATION
        .replace_all(value, |_: &regex::Captures<'_>| {
            let placeholder = format!("X__{i}");
            i += 1;
            placeholder
        })
        .into_owned()
}

/// ref: base_translator.rb#restore_interpolations. The variable check is ours:
/// accepted diff 34.
fn restore(source: &str, translated: &str) -> Result<String, String> {
    let originals: Vec<&str> = INTERPOLATION
        .find_iter(source)
        .map(|m| m.as_str())
        .collect();
    let restored = if originals.is_empty() {
        translated.to_string()
    } else {
        let mut unknown = None;
        let out = PLACEHOLDER.replace_all(translated, |c: &regex::Captures<'_>| {
            match c[1].parse::<usize>().ok().and_then(|i| originals.get(i)) {
                Some(original) => (*original).to_string(),
                None => {
                    unknown = Some(c[0].to_string());
                    c[0].to_string()
                }
            }
        });
        if let Some(placeholder) = unknown {
            return Err(format!(
                "`{source}` came back as `{translated}`, with the unknown placeholder `{placeholder}`"
            ));
        }
        out.into_owned()
    };
    let expected: BTreeSet<&str> = variables(source).into_iter().collect();
    let got: BTreeSet<&str> = variables(&restored).into_iter().collect();
    if expected != got {
        return Err(format!(
            "`{source}` came back as `{restored}`, with other interpolation variables"
        ));
    }
    Ok(restored)
}

/// Adds the translations to `store` and plans the files that receive them.
/// Nothing is written.
///
/// # Errors
///
/// A key cannot be routed, or the write plan fails one of its guards.
pub fn merge(
    cfg: &Config,
    store: &mut Store,
    translated: Vec<Pending>,
) -> Result<NormalizeReport, String> {
    let pattern = PatternRouter::new(cfg);
    let mut routed = Vec::with_capacity(translated.len());
    {
        let conservative = ConservativeRouter::new(&pattern, store);
        for Pending { locale, leaves } in translated {
            if leaves.is_empty() {
                continue;
            }
            let mut out = Vec::with_capacity(leaves.len());
            for leaf in leaves {
                let path = if cfg.data.router == Router::Pattern {
                    pattern.route_key(&locale, &leaf.key)?
                } else {
                    conservative.route_key(&locale, &leaf.key)?
                };
                out.push(Leaf {
                    path: Arc::from(path.as_path()),
                    ..leaf
                });
            }
            routed.push((locale, out));
        }
    }
    let mut locales = Vec::with_capacity(routed.len());
    let mut touched = HashSet::new();
    for (locale, leaves) in routed {
        let keys: HashSet<String> = leaves.iter().map(|l| l.key.clone()).collect();
        let tree = store
            .tree(&locale)
            .ok_or_else(|| format!("locale `{locale}` has no data"))?
            .extended(leaves);
        store.trees.insert(locale.clone(), tree);
        for dest in route::route_filtered(cfg, store, &locale, false, &|k| keys.contains(k))? {
            touched.insert(dest.path);
        }
        locales.push(locale);
    }
    let mut report = normalize::plan(cfg, store, &locales, false)?;
    report.changes.retain(|c| touched.contains(&c.path));
    Ok(report)
}

pub fn rows(pending: &[Pending]) -> Vec<KeyRow> {
    pending
        .iter()
        .flat_map(|p| {
            p.leaves.iter().map(|leaf| KeyRow {
                locale: p.locale.clone(),
                key: leaf.key.clone(),
                value: Some(leaf.value.to_display_string()),
                reason: None,
            })
        })
        .collect()
}

pub fn to_text(rows: &[KeyRow], written: bool) -> String {
    let (title, column) = if written {
        ("Translated keys", "Translation")
    } else {
        ("Missing keys to translate", "Base value")
    };
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.locale.clone(),
                r.key.clone(),
                r.value.clone().unwrap_or_default(),
            ]
        })
        .collect();
    render_table(title, &["Locale", "Key", column], &cells)
}

/// # Errors
///
/// The rows do not serialize.
pub fn to_json(session: &Session, rows: &[KeyRow], written: bool) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "check": "translate-missing",
        "written": written,
        "config_digest": session.cfg.digest,
        "locales": session.locales,
        "keys": rows,
    }))
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn leaf(key: &str, value: Value) -> Leaf {
        Leaf {
            key: key.to_string(),
            value,
            depth: 1,
            path: Arc::from(Path::new("config/locales/en.yml")),
            odd_segments: None,
        }
    }

    fn s(v: &str) -> Value {
        Value::Str(v.to_string())
    }

    fn upcase(batch: &[String]) -> Vec<String> {
        batch.iter().map(|v| v.to_uppercase()).collect()
    }

    #[test]
    fn interpolations_travel_as_numbered_placeholders() {
        assert_eq!(protect("Hi %{name}, %{count} new"), "Hi X__0, X__1 new");
        assert_eq!(
            restore("Hi %{name}, %{count} new", "X__1 uutta, hei X__0").unwrap(),
            "%{count} uutta, hei %{name}"
        );
    }

    #[test]
    fn a_lost_or_unknown_placeholder_fails_the_value() {
        assert!(restore("Hi %{name}", "Hei").is_err());
        assert!(restore("Hi %{name}", "Hei X__3").is_err());
        assert!(restore("Hi", "Hei %{name}").is_err());
    }

    #[test]
    fn collections_are_translated_item_by_item_and_other_scalars_pass_through() {
        let leaves = [
            leaf("days", Value::Seq(vec![s("mon"), Value::Nil, s("tue")])),
            leaf("limit", Value::Plain("5".into())),
            leaf("empty", s("")),
            leaf("flag", Value::Bool(true)),
            leaf(
                "nested",
                Value::Seq(vec![Value::Map(vec![("a".into(), s("x"))])]),
            ),
        ];
        let mut calls = 0;
        let out = translate(&leaves, |batch| {
            calls += 1;
            Ok(upcase(batch))
        });
        assert_eq!(calls, 1);
        assert!(out.failures.is_empty());
        let values: Vec<&Value> = out.leaves.iter().map(|l| &l.value).collect();
        assert_eq!(
            values,
            [
                &Value::Seq(vec![s("MON"), Value::Nil, s("TUE")]),
                &Value::Plain("5".into()),
                &s(""),
                &Value::Bool(true),
                &Value::Seq(vec![Value::Map(vec![("a".into(), s("X"))])]),
            ]
        );
    }

    #[test]
    fn batches_hold_at_most_fifty_strings_and_never_split_a_key() {
        let mut leaves: Vec<Leaf> = (0..49).map(|i| leaf(&format!("k{i:02}"), s("v"))).collect();
        leaves.push(leaf("list", Value::Seq(vec![s("a"), s("b")])));
        leaves.push(leaf("last", s("v")));
        let mut sizes = Vec::new();
        let out = translate(&leaves, |batch| {
            sizes.push(batch.len());
            Ok(upcase(batch))
        });
        assert_eq!(sizes, [49, 3]);
        assert_eq!(out.leaves.len(), 51);
    }

    #[test]
    fn a_failed_batch_drops_only_its_own_keys() {
        let leaves: Vec<Leaf> = (0..60).map(|i| leaf(&format!("k{i:02}"), s("v"))).collect();
        let mut call = 0;
        let out = translate(&leaves, |batch| {
            call += 1;
            if call == 1 {
                Ok(vec!["short".into()])
            } else {
                Ok(upcase(batch))
            }
        });
        assert_eq!(out.failures.len(), 1);
        assert_eq!(out.failures[0].keys.len(), 50);
        assert!(
            out.failures[0].error.contains("received 1"),
            "{}",
            out.failures[0].error
        );
        assert_eq!(out.leaves.len(), 10);
        assert_eq!(out.leaves[0].key, "k50");
    }

    #[test]
    fn pending_takes_every_plural_form_of_a_missing_key() {
        let root = std::env::temp_dir().join("i18n-tasks-rs-translate-pending");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("config/locales")).unwrap();
        std::fs::write(
            root.join("config/locales/en.yml"),
            "en:\n  apples:\n    one: One\n    other: Many\n  hi: Hi\n  skip:\n    me: x\n",
        )
        .unwrap();
        std::fs::write(root.join("config/locales/de.yml"), "de:\n  hi: Hallo\n").unwrap();
        let cfg = Config::parse(
            "base_locale: en\nlocales: [en, de]\ndata:\n  read:\n    - config/locales/%{locale}.yml\nignore_missing:\n  - skip.*\n",
            &root.join("config/i18n-tasks.yml"),
            root.clone(),
        )
        .unwrap();
        let store = Store::load(&cfg).unwrap();
        let pending = pending(&cfg, &store, &["en".into(), "de".into()]);
        let keys: Vec<(&str, &str)> = pending
            .iter()
            .flat_map(|p| p.leaves.iter().map(|l| (p.locale.as_str(), l.key.as_str())))
            .collect();
        assert_eq!(keys, [("de", "apples.one"), ("de", "apples.other")]);
        let _ = std::fs::remove_dir_all(&root);
    }
}

//! `data` and `data-merge`. Accepted diff 35.
//!
//! ref: lib/i18n/tasks/command/commands/data.rb:77-95
//! ref: lib/i18n/tasks/data/file_system_base.rb#merge!

use crate::config::Config;
use crate::data::emit::{Tree, emit_locale};
use crate::data::load::{Leaf, Store, Value};
use crate::data::route;
use crate::keys::parent_key;
use crate::report::normalize::{self, NormalizeReport};
use crate::report::put;
use serde_json::{Map, Value as Json};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

/// The gem's `-f json`: each locale's whole tree, sorted at every level. A
/// locale without data is `{}`.
///
/// ref: lib/i18n/tasks/command/options/data.rb#print_forest
pub fn tree(store: &Store, locales: &[String]) -> Json {
    let mut out = Map::new();
    for locale in locales {
        out.insert(locale.clone(), Json::Object(Map::new()));
        for leaf in store.tree(locale).into_iter().flat_map(|t| &t.leaves) {
            put(&mut out, locale, &leaf.segments(), leaf.value.to_json());
        }
    }
    Json::Object(out)
}

pub fn to_yaml(store: &Store, locales: &[String]) -> String {
    locales
        .iter()
        .map(|locale| {
            let mut out = Tree::new();
            for leaf in store.tree(locale).into_iter().flat_map(|t| &t.leaves) {
                out.insert_segments(&leaf.segments(), leaf.value.clone());
            }
            out.sort();
            emit_locale(locale, &out)
        })
        .collect()
}

/// Merges `input`, `{locale => tree}`, into `store` and plans every file of
/// the merged locales, as the gem's `set` writes them. Returns the plan and
/// the merged locales. Nothing is written.
///
/// A given leaf replaces the value at its key, except where the data holds a
/// mapping, which wins. A given mapping replaces a leaf in its way.
///
/// ref: lib/i18n/tasks/data/tree/siblings.rb#merge_node!
///
/// # Errors
///
/// The input is not a mapping of known locales to trees, a key cannot be
/// routed, or the write plan fails one of its guards.
pub fn merge(
    cfg: &Config,
    store: &mut Store,
    input: &Json,
) -> Result<(NormalizeReport, Vec<String>), String> {
    let Json::Object(roots) = input else {
        return Err("data-merge expects a JSON object of locale to tree".into());
    };
    for (locale, root) in roots {
        let Some(tree) = store.tree(locale) else {
            return Err(format!("unknown locale `{locale}`"));
        };
        if !root.is_object() {
            return Err(format!(
                "`{locale}` holds a value. data-merge expects a tree under each locale"
            ));
        }
        let mut leaves = Vec::new();
        flatten(root, &mut Vec::new(), &mut leaves)?;
        leaves.retain(|leaf| !tree.is_interior(&leaf.key));
        let ancestors: HashSet<String> = leaves
            .iter()
            .flat_map(|leaf| std::iter::successors(parent_key(&leaf.key), |k| parent_key(k)))
            .map(str::to_string)
            .collect();
        let leaves = route::route_leaves(cfg, store, locale, leaves)?;
        let merged = tree.replaced(|leaf| !ancestors.contains(&leaf.key), leaves);
        store.trees.insert(locale.clone(), merged);
    }
    let locales: Vec<String> = roots.keys().cloned().collect();
    let report = normalize::plan(cfg, store, &locales, false)?;
    Ok((report, locales))
}

fn flatten(node: &Json, prefix: &mut Vec<String>, out: &mut Vec<Leaf>) -> Result<(), String> {
    if let Json::Object(entries) = node {
        for (key, value) in entries {
            if key.contains('.') {
                return Err(format!(
                    "`{key}` holds a dot. data-merge cannot tell it from nested keys"
                ));
            }
            prefix.push(key.clone());
            flatten(value, prefix, out)?;
            prefix.pop();
        }
        return Ok(());
    }
    out.push(Leaf::new(prefix, to_value(node), Arc::from(Path::new(""))));
    Ok(())
}

fn to_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Plain(n.to_string()),
        Json::String(s) => Value::Str(s.clone()),
        Json::Array(items) => Value::Seq(items.iter().map(to_value).collect()),
        Json::Object(entries) => Value::Map(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), to_value(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(json: &str) -> Vec<Leaf> {
        let mut out = Vec::new();
        flatten(
            &serde_json::from_str(json).unwrap(),
            &mut Vec::new(),
            &mut out,
        )
        .unwrap();
        out
    }

    #[test]
    fn json_values_become_the_values_psych_would_read() {
        let got: Vec<(String, Value)> =
            leaves(r#"{"s":"5","n":5,"f":1.5,"b":false,"z":null,"a":["x",{"k":1}],"e":{}}"#)
                .into_iter()
                .map(|l| (l.key, l.value))
                .collect();
        assert_eq!(
            got,
            [
                (
                    "a".to_string(),
                    Value::Seq(vec![
                        Value::Str("x".into()),
                        Value::Map(vec![("k".into(), Value::Plain("1".into()))]),
                    ])
                ),
                ("b".to_string(), Value::Bool(false)),
                ("f".to_string(), Value::Plain("1.5".into())),
                ("n".to_string(), Value::Plain("5".into())),
                ("s".to_string(), Value::Str("5".into())),
                ("z".to_string(), Value::Nil),
            ]
        );
    }

    #[test]
    fn a_nested_key_has_its_depth() {
        let got = leaves(r#"{"a":{"b":{"c":"x"}}}"#);
        assert_eq!((got[0].key.as_str(), got[0].depth), ("a.b.c", 3));
    }
}

//! ref: lib/i18n/tasks/command/commands/data.rb:77-95
//! The goldens are what i18n-tasks 1.1.2 printed and wrote: `data` over
//! `tests/fixtures/missing_json`, `data-merge` over a copy of
//! `tests/fixtures/data_merge` fed `tests/golden/data_merge/input.json`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_i18n-tasks-rs");

fn golden(name: &str) -> String {
    std::fs::read_to_string(format!("tests/golden/{name}")).unwrap()
}

fn run(root: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(BIN)
        .args(args)
        .arg("-c")
        .arg(root.join("config/i18n-tasks.yml"))
        .arg("--root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn data(args: &[&str]) -> (i32, String) {
    let mut all = vec!["data"];
    all.extend(args);
    let (code, out, _) = run(Path::new("tests/fixtures/missing_json"), &all, "");
    (code, out)
}

struct Project {
    root: PathBuf,
}

impl Project {
    fn new(name: &str) -> Project {
        let root = std::env::temp_dir().join(format!("i18n-tasks-rs-data-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let locales = root.join("config/locales");
        std::fs::create_dir_all(&locales).unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        let fixture = Path::new("tests/fixtures/data_merge/config");
        std::fs::copy(
            fixture.join("i18n-tasks.yml"),
            root.join("config/i18n-tasks.yml"),
        )
        .unwrap();
        for entry in std::fs::read_dir(fixture.join("locales")).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, locales.join(path.file_name().unwrap())).unwrap();
        }
        Project { root }
    }

    fn write(&self, rel: &str, contents: &str) -> &Project {
        std::fs::write(self.root.join(rel), contents).unwrap();
        self
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join("config/locales").join(name)).unwrap()
    }

    fn merge(&self, args: &[&str], stdin: &str) -> (i32, String, String) {
        let mut all = vec!["data-merge"];
        all.extend(args);
        run(&self.root, &all, stdin)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture_locale(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/data_merge/config/locales/{name}")).unwrap()
}

#[test]
fn data_prints_every_locale_as_the_gem_does() {
    assert_eq!(data(&["-f", "json"]), (0, golden("data_all.json")));
}

#[test]
fn data_prints_only_the_locales_asked_for() {
    assert_eq!(data(&["de", "-f", "json"]), (0, golden("data_de.json")));
}

#[test]
fn data_text_is_the_locale_files_yaml() {
    let (code, out) = data(&["ru"]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "---\nru:\n  apples:\n    one: \"%{count} яблоко\"\n    other: \"%{count} яблок\"\n  zeta: Я\n"
    );
}

#[test]
fn data_merge_prints_and_writes_what_the_gem_does() {
    let p = Project::new("gem");
    let (code, out, err) = p.merge(&["-f", "json"], &golden("data_merge/input.json"));
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, golden("data_merge/output.json"));
    assert_eq!(p.read("ru.yml"), golden("data_merge/ru.yml"));
    assert_eq!(p.read("devise.ru.yml"), golden("data_merge/devise.ru.yml"));
    assert_eq!(p.read("en.yml"), fixture_locale("en.yml"));
    assert_eq!(p.read("devise.en.yml"), fixture_locale("devise.en.yml"));
}

#[test]
fn data_merge_replaces_a_leaf_with_the_mapping_given_for_it() {
    let p = Project::new("leaf-to-map");
    let (code, out, _) = p.merge(&["-f", "json"], r#"{"ru":{"title":{"x":"Икс"}}}"#);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "{\"ru\":{\"apples\":{\"one\":\"%{count} яблоко\"},\"devise\":{\"items\":{\"one\":\"один\"}},\"menu\":{\"home\":\"Главная\"},\"title\":{\"x\":\"Икс\"}}}\n"
    );
    assert_eq!(
        p.read("ru.yml"),
        "---\nru:\n  apples:\n    one: \"%{count} яблоко\"\n  menu:\n    home: Главная\n  title:\n    x: Икс\n"
    );
}

#[test]
fn data_merge_normalizes_the_merged_locales_files_even_with_nothing_new() {
    let p = Project::new("noop");
    let (code, _, _) = p.merge(&["-f", "json"], r#"{"ru":{"title":"Заголовок"}}"#);
    assert_eq!(code, 0);
    assert_eq!(
        p.read("ru.yml"),
        "---\nru:\n  apples:\n    one: \"%{count} яблоко\"\n  menu:\n    home: Главная\n  title: Заголовок\n"
    );
    assert_eq!(p.read("en.yml"), fixture_locale("en.yml"));
}

#[test]
fn data_merge_deletes_a_file_it_empties_only_when_allowed() {
    let p = Project::new("vacate");
    let input = r#"{"ru":{"devise":{"items":{"one":{"x":"Икс"}}}}}"#;
    let (code, out, err) = p.merge(&["-f", "json"], input);
    assert_eq!((code, out.as_str()), (2, ""), "{err}");
    assert!(err.contains("devise.ru.yml"), "{err}");
    assert_eq!(p.read("devise.ru.yml"), fixture_locale("devise.ru.yml"));

    let (code, _, err) = p.merge(&["-f", "json", "--allow-delete"], input);
    assert_eq!(code, 0, "{err}");
    assert!(!p.root.join("config/locales/devise.ru.yml").exists());
    assert!(p.read("ru.yml").contains("      one:\n        x: Икс\n"));
}

#[test]
fn data_merge_under_the_pattern_router_deletes_a_file_a_moved_key_empties_only_when_allowed() {
    let p = Project::new("pattern-move");
    p.write(
        "config/i18n-tasks.yml",
        "base_locale: en\nlocales: [en, ru]\ndata:\n  router: pattern_router\n  read:\n    - config/locales/%{locale}.yml\n    - config/locales/**/*.%{locale}.yml\n  write:\n    - config/locales/%{locale}.yml\nsearch:\n  paths: [app/]\n",
    );
    let input = r#"{"ru":{"devise":{"items":{"one":"одна"}}}}"#;
    let (code, _, err) = p.merge(&["-f", "json"], input);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("devise.ru.yml"), "{err}");
    assert_eq!(p.read("devise.ru.yml"), fixture_locale("devise.ru.yml"));

    let (code, _, err) = p.merge(&["-f", "json", "--allow-delete"], input);
    assert_eq!(code, 0, "{err}");
    assert!(!p.root.join("config/locales/devise.ru.yml").exists());
    assert!(p.read("ru.yml").contains("    items:\n      one: одна\n"));
}

#[test]
fn data_merge_refuses_a_key_that_collides_with_a_key_holding_a_dot() {
    let p = Project::new("dotted-existing");
    let ru = "ru:\n  a.b: old\n  title: Заголовок\n";
    p.write("config/locales/ru.yml", ru);
    for input in [
        r#"{"ru":{"a":{"b":"new"}}}"#,
        r#"{"ru":{"a":"new"}}"#,
        r#"{"ru":{"a":{"b":{"c":"new"}}}}"#,
    ] {
        let (code, _, err) = p.merge(&["-f", "json"], input);
        assert_eq!(code, 2, "{input}: {err}");
        assert!(err.contains("a.b"), "{input}: {err}");
        assert_eq!(p.read("ru.yml"), ru, "{input}");
    }
}

#[test]
fn data_merge_refuses_a_key_holding_a_dot() {
    let p = Project::new("dotted");
    let (code, _, err) = p.merge(&["-f", "json"], r#"{"ru":{"title.sub":"x"}}"#);
    assert_eq!(code, 2);
    assert!(err.contains("title.sub"), "{err}");
    assert_eq!(p.read("ru.yml"), fixture_locale("ru.yml"));
}

#[test]
fn data_merge_reads_json_only() {
    let p = Project::new("text");
    let (code, _, err) = p.merge(&[], r#"{"ru":{"title":"x"}}"#);
    assert_eq!(code, 2);
    assert!(err.contains("-f json"), "{err}");
    assert_eq!(p.read("ru.yml"), fixture_locale("ru.yml"));
}

#[test]
fn data_merge_takes_no_locales() {
    let p = Project::new("locales");
    let (code, _, err) = p.merge(&["ru", "-f", "json"], r#"{"ru":{"title":"x"}}"#);
    assert_eq!(code, 2);
    assert!(err.contains("stdin"), "{err}");
    assert_eq!(p.read("ru.yml"), fixture_locale("ru.yml"));
}

#[test]
fn data_merge_refuses_bad_input_before_writing() {
    let p = Project::new("bad");
    for input in [
        "",
        "not json",
        r#"["ru"]"#,
        r#"{"ru":"x"}"#,
        r#"{"xx":{"a":"b"}}"#,
        r#"{"ru":{"title":"x"},"de":{"a":"b"}}"#,
    ] {
        let (code, out, err) = p.merge(&["-f", "json"], input);
        assert_eq!((code, out.as_str()), (2, ""), "{input}: {err}");
        assert_eq!(p.read("ru.yml"), fixture_locale("ru.yml"), "{input}");
    }
}

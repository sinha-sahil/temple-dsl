//! Goldens under `tests/compat/` come from 0.3.0; regenerate them only on a
//! deliberate format change:
//! `TEMPLE_BLESS=1 cargo test --all-features --test compat`.
#![cfg(feature = "json")]

use std::fs;
use std::path::{Path, PathBuf};
use temple_dsl::Template;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn corpus() -> Vec<(String, String, Option<serde_json::Value>)> {
    let mut out = Vec::new();
    for dir in ["samples", "examples/data"] {
        let mut entries: Vec<_> = fs::read_dir(root().join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "temple"))
            .collect();
        entries.sort();
        for path in entries {
            let name = format!(
                "{}__{}",
                dir.replace('/', "_"),
                path.file_stem().unwrap().to_string_lossy()
            );
            let src = fs::read_to_string(&path).unwrap();
            let input = path.with_extension("json");
            let input = input
                .exists()
                .then(|| serde_json::from_str(&fs::read_to_string(input).unwrap()).unwrap());
            out.push((name, src, input));
        }
    }
    out
}

fn golden(name: &str, ext: &str) -> PathBuf {
    root().join("tests/compat").join(format!("{name}.{ext}"))
}

#[test]
fn golden_blobs_outputs_and_formatting_are_unchanged() {
    let bless = std::env::var_os("TEMPLE_BLESS").is_some();
    let corpus = corpus();
    assert!(corpus.len() >= 14, "corpus shrank: {}", corpus.len());

    for (name, src, input) in corpus {
        let template = Template::compile(&src).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let blob = template.to_bytes();
        let formatted = Template::format(&src).unwrap();
        let output = input.map(|i| {
            serde_json::to_string_pretty(&serde_json::Value::from(
                template.render_value(i).unwrap(),
            ))
            .unwrap()
        });

        if bless {
            fs::write(golden(&name, "blob"), &blob).unwrap();
            fs::write(golden(&name, "fmt"), &formatted).unwrap();
            if let Some(o) = &output {
                fs::write(golden(&name, "out.json"), o).unwrap();
            }
            continue;
        }

        let old_blob = fs::read(golden(&name, "blob")).unwrap();
        assert_eq!(blob, old_blob, "{name}: compiled blob changed");
        assert_eq!(
            formatted,
            fs::read_to_string(golden(&name, "fmt")).unwrap(),
            "{name}: formatter output changed"
        );

        // An old blob must still load and render the same output.
        let loaded = Template::from_bytes(&old_blob).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        if let Some(expected) = output {
            let input_path = if name.starts_with("examples_data__") {
                root().join("examples/data").join(format!(
                    "{}.json",
                    name.trim_start_matches("examples_data__")
                ))
            } else {
                unreachable!()
            };
            let input: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(input_path).unwrap()).unwrap();
            let rendered = serde_json::to_string_pretty(&serde_json::Value::from(
                loaded.render_value(input).unwrap(),
            ))
            .unwrap();
            assert_eq!(
                rendered, expected,
                "{name}: output from the old blob changed"
            );
            assert_eq!(
                rendered,
                fs::read_to_string(golden(&name, "out.json")).unwrap(),
                "{name}: output changed"
            );
        }
    }
}

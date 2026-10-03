//! `dist/README.md` is the only README the marketplace listing shows, and the
//! release publishes a much smaller tree than this repository. A link to
//! `docs/` or `Cargo.toml` reads fine here and resolves to nothing there.
//!
//! The published set is derived from `release.yml` rather than written down
//! twice, so changing the workflow's allowlist moves this test with it.

use std::collections::HashSet;
use std::path::Path;

const RELEASE_WORKFLOW: &str = ".github/workflows/release.yml";
const PUBLISHED_README: &str = "dist/README.md";

/// Every path the release workflow copies into the distribution repository.
///
/// Reads the two `cp` lines instead of hardcoding their arguments: the whole
/// point is to notice when that allowlist changes.
fn published_paths(repo: &Path) -> HashSet<String> {
    let workflow = std::fs::read_to_string(repo.join(RELEASE_WORKFLOW))
        .unwrap_or_else(|e| panic!("cannot read {RELEASE_WORKFLOW}: {e}"));
    let copy_line = workflow
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("cp -- ") && line.contains("manifest.json"))
        .unwrap_or_else(|| {
            panic!("no root `cp --` line in {RELEASE_WORKFLOW}; the allowlist moved")
        });

    let mut published = HashSet::new();
    for token in copy_line
        .trim_start_matches("cp -- ")
        .split_whitespace()
        .filter(|t| !t.contains("$DIST"))
    {
        if token == "*.qml" {
            let entries = std::fs::read_dir(repo).expect("cannot list the repository root");
            published.extend(
                entries
                    .flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|name| name.ends_with(".qml")),
            );
        } else {
            published.insert(token.to_string());
        }
    }
    // dist/README.md lands as README.md, and the binary is copied separately.
    published.insert("README.md".to_string());
    published.insert("bin/perfo".to_string());
    published
}

/// Local paths the published README points at: markdown links and images, plus
/// file names in backticks, which read as references just as much as a link.
fn referenced_paths(readme: &str) -> Vec<String> {
    const FILE_SUFFIXES: [&str; 7] = [".md", ".json", ".png", ".qml", ".yml", ".toml", ".sh"];
    let mut found = Vec::new();

    for (i, _) in readme.match_indices("](") {
        let rest = &readme[i + 2..];
        let Some(end) = rest.find(')') else { continue };
        found.push(rest[..end].to_string());
    }
    for chunk in readme.split('`').skip(1).step_by(2) {
        if FILE_SUFFIXES.iter().any(|s| chunk.ends_with(s)) || chunk == "bin/perfo" {
            found.push(chunk.to_string());
        }
    }

    found.retain(|p| {
        !p.starts_with("http://")
            && !p.starts_with("https://")
            && !p.starts_with('#')
            && !p.starts_with('<')
            && !p.is_empty()
    });
    found.sort();
    found.dedup();
    found
}

#[test]
fn published_readme_only_points_at_published_files() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let published = published_paths(repo);
    let readme = std::fs::read_to_string(repo.join(PUBLISHED_README))
        .unwrap_or_else(|e| panic!("cannot read {PUBLISHED_README}: {e}"));

    let referenced = referenced_paths(&readme);
    assert!(
        !referenced.is_empty(),
        "parsed no references out of {PUBLISHED_README}; the parser is broken, not the README"
    );

    let dangling: Vec<&String> = referenced
        .iter()
        .filter(|p| !published.contains(p.as_str()))
        .collect();
    assert!(
        dangling.is_empty(),
        "{PUBLISHED_README} points at files the release does not publish: {dangling:?}\n\
         published set is {published:?}"
    );
}

#[test]
fn the_development_readme_is_not_the_published_one() {
    // They were the same file once, which is how the dead links got shipped.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dev = std::fs::read_to_string(repo.join("README.md")).expect("cannot read README.md");
    let published =
        std::fs::read_to_string(repo.join(PUBLISHED_README)).expect("cannot read dist/README.md");
    assert_ne!(dev, published);
}

#[test]
fn published_readme_documents_the_marketplace_install_command() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme =
        std::fs::read_to_string(repo.join(PUBLISHED_README)).expect("cannot read dist/README.md");
    assert!(
        readme.contains("omarchy plugin add https://github.com/VitorHolandaI/perfo.git --enable"),
        "the listing's only install path has to be in the README it shows"
    );
    // The listing carries a manual-setup override precisely because these
    // used to be the documented way in. They belong in the dev README.
    for manual in ["export PERFO_BIN", "mkdir -p \"$HOME/.config/omarchy"] {
        assert!(
            !readme.contains(manual),
            "{PUBLISHED_README} still documents a manual install step: {manual:?}"
        );
    }
}

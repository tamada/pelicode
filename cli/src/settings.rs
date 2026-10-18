//! Settings files: `.pelicode.yaml`, `.pelicode.yml`, `.pelicode.json5` or
//! `.pelicode.json`, found in the current directory or above it, up to the
//! root of the git repository.
//!
//! Settings given on the command line win over the settings file, which wins
//! over the built-in defaults.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use globset::{GlobSet, GlobSetBuilder};
use pelicode_core::Language;
use serde::Deserialize;

use crate::scan::StopThreshold;

/// Names of settings files, in the order they are looked for.
pub const FILE_NAMES: [&str; 4] = [
    ".pelicode.yaml",
    ".pelicode.yml",
    ".pelicode.json5",
    ".pelicode.json",
];

/// The only version of the settings format so far.
const VERSION: u32 = 1;

/// Contents of a settings file.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Version of the settings format; must be 1.
    pub version: u32,
    /// Database file, relative to the settings file, or several for `scan`.
    #[serde(default)]
    pub database: Option<Databases>,
    /// Settings of `pelicode scan`.
    #[serde(default)]
    pub scan: ScanSettings,
    /// Settings of `pelicode crawl`.
    #[serde(default)]
    pub crawl: CrawlSettings,
}

/// Settings of `pelicode scan`; see its options.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanSettings {
    /// Languages to check; every supported language when absent.
    pub languages: Option<Vec<Language>>,
    pub threshold: Option<f64>,
    pub min_tokens: Option<usize>,
    pub max_candidates: Option<usize>,
    pub min_shared: Option<usize>,
    /// A number of packages, or a percentage such as `"10%"`.
    pub stop_packages: Option<StopValue>,
    pub idf: Option<bool>,
    pub fail_on_match: Option<bool>,
    /// Leave out candidates found only in the dependencies of the checked code
    /// (true when absent).
    pub exclude_dependencies: Option<bool>,
    #[serde(default)]
    pub exclude: Exclude,
}

/// What `pelicode scan` leaves out.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exclude {
    /// Packages never reported, as `ECOSYSTEM:NAME`.
    #[serde(default)]
    pub packages: Vec<String>,
    /// Files not checked, as glob patterns over their reported paths.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Settings of `pelicode crawl`; see its options.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrawlSettings {
    pub contact: Option<String>,
    pub delay_ms: Option<u64>,
}

/// `stop_packages` as written: a number, or a string such as `"10%"`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StopValue {
    Count(usize),
    Text(String),
}

impl StopValue {
    fn threshold(&self) -> Result<StopThreshold> {
        match self {
            StopValue::Count(n) => Ok(StopThreshold::Count(*n)),
            StopValue::Text(s) => s.parse().map_err(anyhow::Error::msg),
        }
    }
}

/// One database, or a list of them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Databases {
    One(PathBuf),
    Many(Vec<PathBuf>),
}

/// A settings file and where it was found.
#[derive(Debug)]
pub struct Loaded {
    pub path: PathBuf,
    pub settings: Settings,
}

impl Loaded {
    /// The databases of the settings, relative to the settings file; empty
    /// when there is none.
    pub fn databases(&self) -> Vec<PathBuf> {
        let list = match &self.settings.database {
            None => return Vec::new(),
            Some(Databases::One(db)) => std::slice::from_ref(db),
            Some(Databases::Many(dbs)) => dbs.as_slice(),
        };
        list.iter()
            .map(|db| match self.path.parent() {
                Some(dir) if db.is_relative() && !crate::oci::is_reference(db) => dir.join(db),
                _ => db.clone(),
            })
            .collect()
    }

    /// The stop threshold of the settings, if any.
    pub fn stop_packages(&self) -> Result<Option<StopThreshold>> {
        self.settings
            .scan
            .stop_packages
            .as_ref()
            .map(StopValue::threshold)
            .transpose()
    }
}

/// The settings to use: those of `explicit` when given, none with
/// `disabled`, otherwise the settings file found from the current directory.
pub fn load(explicit: Option<&Path>, disabled: bool) -> Result<Option<Loaded>> {
    let path = match (explicit, disabled) {
        (_, true) => return Ok(None),
        (Some(path), false) => path.to_path_buf(),
        (None, false) => match find(&std::env::current_dir()?)? {
            Some(path) => path,
            None => return Ok(None),
        },
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let settings = parse(&path, &text)?;
    Ok(Some(Loaded { path, settings }))
}

/// The settings file in `start` or the nearest directory above it, up to the
/// root of the git repository containing `start`.
pub fn find(start: &Path) -> Result<Option<PathBuf>> {
    for dir in start.ancestors() {
        let found: Vec<PathBuf> = FILE_NAMES
            .iter()
            .map(|name| dir.join(name))
            .filter(|path| path.is_file())
            .collect();
        match found.as_slice() {
            [] => {}
            [path] => return Ok(Some(path.clone())),
            _ => bail!(
                "several settings files in {}: {}; keep one",
                dir.display(),
                found
                    .iter()
                    .filter_map(|p| p.file_name()?.to_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    Ok(None)
}

/// Parses and validates settings; the format follows the file extension.
pub fn parse(path: &Path, text: &str) -> Result<Settings> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let settings: Settings = match extension {
        "yaml" | "yml" => serde_saphyr::from_str(text).map_err(anyhow::Error::msg),
        "json5" | "json" => json5::from_str(text).map_err(anyhow::Error::msg),
        _ => bail!(
            "{}: unknown settings format; use .yaml, .yml, .json5 or .json",
            path.display()
        ),
    }
    .with_context(|| format!("invalid settings in {}", path.display()))?;
    validate(&settings).with_context(|| format!("invalid settings in {}", path.display()))?;
    Ok(settings)
}

fn validate(settings: &Settings) -> Result<()> {
    if settings.version != VERSION {
        bail!(
            "unsupported version {}; expected {VERSION}",
            settings.version
        );
    }
    let scan = &settings.scan;
    if let Some(t) = scan.threshold
        && !(0.0..=1.0).contains(&t)
    {
        bail!("scan.threshold must be between 0 and 1, got {t}");
    }
    if let Some(stop) = &scan.stop_packages {
        stop.threshold().context("scan.stop_packages")?;
    }
    for package in &scan.exclude.packages {
        parse_package(package).context("scan.exclude.packages")?;
    }
    glob_set(&scan.exclude.paths).context("scan.exclude.paths")?;
    Ok(())
}

/// `ECOSYSTEM:NAME` as a pair.
pub fn parse_package(spec: &str) -> Result<(String, String)> {
    match spec.split_once(':') {
        Some((eco, name)) if !eco.is_empty() && !name.is_empty() => {
            Ok((eco.to_string(), name.to_string()))
        }
        _ => bail!("expected ECOSYSTEM:NAME, got {spec}"),
    }
}

/// Glob patterns matched against `/`-separated paths, where `*` stays within a
/// directory and `**` crosses directories.
pub fn glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid pattern {pattern}"))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
version: 1
database: data/pelicode.db
scan:
  languages: [python, rust]
  threshold: 0.6
  stop_packages: 10%
  idf: true
  exclude:
    packages: ["local:me"]
    paths: ["vendor/**", "**/testdata/**"]
crawl:
  contact: https://example.org/about
"#;

    const JSON5: &str = r#"{
  // the same settings in JSON5
  version: 1,
  database: 'data/pelicode.db',
  scan: {
    languages: ['python', 'rust'],
    threshold: 0.6,
    stop_packages: '10%',
    idf: true,
    exclude: { packages: ['local:me'], paths: ['vendor/**', '**/testdata/**'] },
  },
  crawl: { contact: 'https://example.org/about' },
}"#;

    #[test]
    fn yaml_and_json5_mean_the_same() {
        for (name, text) in [(".pelicode.yaml", YAML), (".pelicode.json5", JSON5)] {
            let s = parse(Path::new(name), text).unwrap();
            assert_eq!(
                s.database,
                Some(Databases::One(PathBuf::from("data/pelicode.db")))
            );
            assert_eq!(
                s.scan.languages,
                Some(vec![Language::Python, Language::Rust])
            );
            assert_eq!(s.scan.threshold, Some(0.6));
            assert!(matches!(
                s.scan.stop_packages.as_ref().unwrap().threshold().unwrap(),
                StopThreshold::Percent(p) if p == 10.0
            ));
            assert_eq!(s.scan.exclude.paths.len(), 2);
            assert_eq!(
                s.crawl.contact.as_deref(),
                Some("https://example.org/about")
            );
        }
    }

    #[test]
    fn stop_packages_may_be_a_number() {
        let s = parse(
            Path::new("a.yml"),
            "version: 1\nscan:\n  stop_packages: 50\n",
        )
        .unwrap();
        assert!(matches!(
            s.scan.stop_packages.unwrap().threshold().unwrap(),
            StopThreshold::Count(50)
        ));
    }

    #[test]
    fn rejects_mistakes() {
        let err =
            |name: &str, text: &str| format!("{:#}", parse(Path::new(name), text).unwrap_err());
        assert!(err("a.yaml", "version: 2\n").contains("unsupported version"));
        assert!(err("a.yaml", "version: 1\nscan:\n  treshold: 0.5\n").contains("treshold"));
        assert!(err("a.yaml", "version: 1\nscan:\n  threshold: 7\n").contains("between 0 and 1"));
        assert!(err("a.yaml", "version: 1\nscan:\n  languages: [cobol]\n").contains("cobol"));
        assert!(
            err("a.json", "{version: 1, scan: {exclude: {packages: ['x']}}}")
                .contains("ECOSYSTEM:NAME")
        );
        assert!(err("a.toml", "version = 1").contains("unknown settings format"));
    }

    #[test]
    fn globs_respect_directories() {
        let set = glob_set(&["vendor/**".into(), "*.gen.py".into()]).unwrap();
        assert!(set.is_match("vendor/a/b.py"));
        assert!(set.is_match("x.gen.py"));
        assert!(!set.is_match("src/x.gen.py"));
        assert!(!set.is_match("src/vendor.py"));
    }

    #[test]
    fn finds_the_nearest_file_within_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let sub = repo.join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        // Outside the repository: never used.
        std::fs::write(dir.path().join(".pelicode.yaml"), "version: 1\n").unwrap();
        assert_eq!(find(&sub).unwrap(), None);

        std::fs::write(repo.join(".pelicode.json5"), "{version: 1}").unwrap();
        assert_eq!(find(&sub).unwrap(), Some(repo.join(".pelicode.json5")));

        std::fs::write(repo.join(".pelicode.yml"), "version: 1\n").unwrap();
        assert!(find(&sub).is_err());
    }

    #[test]
    fn database_is_relative_to_the_file() {
        let loaded = Loaded {
            path: PathBuf::from("/work/repo/.pelicode.yaml"),
            settings: parse(Path::new("a.yaml"), "version: 1\ndatabase: db/p.db\n").unwrap(),
        };
        assert_eq!(loaded.databases(), [PathBuf::from("/work/repo/db/p.db")]);
    }

    #[test]
    fn several_databases() {
        let text =
            "version: 1\ndatabase: [pypi.db, 'oci://quay.io/tama5/pelicode-db:crates-latest']\n";
        for (name, text) in [
            ("a.yaml", text.to_string()),
            ("a.json5", "{version: 1, database: ['pypi.db', 'oci://quay.io/tama5/pelicode-db:crates-latest']}".to_string()),
        ] {
            let loaded = Loaded {
                path: PathBuf::from("/work/repo/.pelicode.yaml"),
                settings: parse(Path::new(name), &text).unwrap(),
            };
            assert_eq!(
                loaded.databases(),
                [
                    PathBuf::from("/work/repo/pypi.db"),
                    PathBuf::from("oci://quay.io/tama5/pelicode-db:crates-latest")
                ]
            );
        }
    }
}

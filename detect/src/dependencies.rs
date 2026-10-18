//! Dependencies of the checked code, read from its lock files and manifests.
//!
//! Code that matches a library the checked project depends on is expected,
//! so [`Detector`](crate::Detector) can leave such matches out
//! ([`Options::dependencies`](crate::Options::dependencies)). The names
//! follow the database: PEP 503 names for PyPI, `group:artifact` for Maven,
//! module paths for Go.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Packages the checked code depends on, and the files they were read from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dependencies {
    /// `(ecosystem, name)` pairs, as in the database.
    pub packages: BTreeSet<(String, String)>,
    /// Lock files and manifests read.
    pub files: Vec<PathBuf>,
}

/// The kinds of files read, by file name.
const FILES: [&str; 10] = [
    "Cargo.lock",
    "go.mod",
    "go.sum",
    "requirements.txt",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "pom.xml",
    "gradle.lockfile",
];

fn is_dependency_file(name: &str) -> bool {
    FILES.contains(&name) || (name.starts_with("requirements") && name.ends_with(".txt"))
}

impl Dependencies {
    /// Reads every lock file and manifest under `root` (or `root` itself when
    /// it is a file). Hidden files and, in Git repositories, files ignored by
    /// `.gitignore` are skipped; so are files that cannot be read or parsed.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_detect::dependencies::Dependencies;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// std::fs::write(dir.path().join("requirements.txt"), "Requests>=2.31\n").unwrap();
    /// std::fs::write(dir.path().join("go.mod"), "module example.com/m\n\nrequire github.com/google/uuid v1.6.0\n").unwrap();
    ///
    /// let deps = Dependencies::find(dir.path());
    /// assert!(deps.contains("pypi", "requests"));
    /// assert!(deps.contains("go", "github.com/google/uuid"));
    /// assert_eq!(deps.files.len(), 2);
    /// ```
    pub fn find(root: &Path) -> Dependencies {
        let mut deps = Dependencies::default();
        let paths: Vec<PathBuf> = if root.is_file() {
            vec![root.to_path_buf()]
        } else {
            ignore::WalkBuilder::new(root)
                .build()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
                .map(|e| e.into_path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(is_dependency_file)
                })
                .collect()
        };
        for path in paths {
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                deps.add_file(name, &text);
                deps.files.push(path);
            }
        }
        deps.files.sort();
        deps
    }

    /// The dependencies of code checked under `root`: [`Dependencies::find`]
    /// under `root`, plus the lock files and manifests directly in each
    /// directory above it, up to the root of the git repository. So checking
    /// `./src` still reads the lock files at the root of the project.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_detect::dependencies::Dependencies;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    /// std::fs::create_dir_all(dir.path().join("src")).unwrap();
    /// std::fs::write(dir.path().join("Gemfile.lock"), "GEM\n  specs:\n    rack (3.1.8)\n").unwrap();
    ///
    /// let deps = Dependencies::find_around(&dir.path().join("src"));
    /// assert!(deps.contains("rubygems", "rack"));
    /// ```
    pub fn find_around(root: &Path) -> Dependencies {
        let mut deps = Dependencies::find(root);
        let start = if root.is_file() {
            root.parent()
        } else {
            Some(root)
        };
        let Some(start) = start.and_then(|p| p.canonicalize().ok()) else {
            return deps;
        };
        if start.join(".git").exists() {
            return deps;
        }
        for dir in start.ancestors().skip(1) {
            let mut above = Dependencies::default();
            for name in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = name.path();
                if path.is_file()
                    && let Some(file) = path.file_name().and_then(|n| n.to_str())
                    && is_dependency_file(file)
                    && let Ok(text) = std::fs::read_to_string(&path)
                {
                    above.add_file(file, &text);
                    above.files.push(path);
                }
            }
            deps.extend(above);
            if dir.join(".git").exists() {
                break;
            }
        }
        deps
    }

    /// Adds the dependencies declared in `text`, the content of a file named
    /// `file_name` (`Cargo.lock`, `go.mod`, `requirements.txt`, ...). Files of
    /// other names, and content that cannot be parsed, add nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_detect::dependencies::Dependencies;
    ///
    /// let mut deps = Dependencies::default();
    /// deps.add_file("Gemfile.lock", "GEM\n  remote: https://rubygems.org/\n  specs:\n    rack (3.1.8)\n");
    /// assert!(deps.contains("rubygems", "rack"));
    /// ```
    pub fn add_file(&mut self, file_name: &str, text: &str) {
        let found = match file_name {
            "Cargo.lock" => cargo_lock(text),
            "go.mod" => go_mod(text),
            "go.sum" => go_sum(text),
            "poetry.lock" | "uv.lock" => toml_packages(text, "pypi"),
            "Pipfile.lock" => pipfile_lock(text),
            "Gemfile.lock" => gemfile_lock(text),
            "pom.xml" => pom_xml(text),
            "gradle.lockfile" => gradle_lockfile(text),
            name if is_dependency_file(name) => requirements(text),
            _ => Vec::new(),
        };
        self.packages.extend(found);
    }

    /// Whether `name` of `ecosystem` is a dependency.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_detect::dependencies::Dependencies;
    ///
    /// let mut deps = Dependencies::default();
    /// deps.add_file("requirements.txt", "typing_extensions==4.12\n");
    /// assert!(deps.contains("pypi", "typing-extensions"));
    /// assert!(!deps.contains("crates", "typing-extensions"));
    /// ```
    pub fn contains(&self, ecosystem: &str, name: &str) -> bool {
        self.packages
            .contains(&(ecosystem.to_string(), name.to_string()))
    }

    /// Merges the dependencies of `other` into these.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_detect::dependencies::Dependencies;
    ///
    /// let mut a = Dependencies::default();
    /// a.add_file("requirements.txt", "six\n");
    /// let mut b = Dependencies::default();
    /// b.add_file("requirements.txt", "idna\n");
    /// a.extend(b);
    /// assert_eq!(a.packages.len(), 2);
    /// ```
    pub fn extend(&mut self, other: Dependencies) {
        self.packages.extend(other.packages);
        self.files.extend(other.files);
        self.files.sort();
        self.files.dedup();
    }
}

fn pair(ecosystem: &str, name: impl Into<String>) -> (String, String) {
    (ecosystem.to_string(), name.into())
}

/// PEP 503 normalized name, as the database records PyPI projects.
fn pypi_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut separator = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            separator = true;
        } else {
            if separator && !out.is_empty() {
                out.push('-');
            }
            separator = false;
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Every `[[package]]` of a Cargo.lock, including the workspace's own crates,
/// so that their published versions are not reported either.
fn cargo_lock(text: &str) -> Vec<(String, String)> {
    let Ok(value) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    package_names(&value)
        .into_iter()
        .map(|name| pair("crates", name))
        .collect()
}

/// `[[package]]` names of a poetry.lock or uv.lock.
fn toml_packages(text: &str, ecosystem: &str) -> Vec<(String, String)> {
    let Ok(value) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    package_names(&value)
        .into_iter()
        .map(|name| pair(ecosystem, pypi_name(&name)))
        .collect()
}

fn package_names(table: &toml::Table) -> Vec<String> {
    table
        .get("package")
        .and_then(|p| p.as_array())
        .map(|packages| {
            packages
                .iter()
                .filter_map(|p| p.get("name")?.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Module paths in the `require` directives of a go.mod.
fn go_mod(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if in_block {
            if line == ")" {
                in_block = false;
            } else if let Some(module) = line.split_whitespace().next() {
                found.push(pair("go", module));
            }
        } else if let Some(rest) = line.strip_prefix("require") {
            let rest = rest.trim();
            if rest == "(" {
                in_block = true;
            } else if let Some(module) = rest.split_whitespace().next() {
                found.push(pair("go", module));
            }
        }
    }
    found
}

/// Module paths of a go.sum.
fn go_sum(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(|module| pair("go", module))
        .collect()
}

/// Project names of a requirements file: `name`, `name==1.0`, `name[extra]>=1`,
/// `name; python_version < "3.12"`; options, URLs and comments are skipped.
fn requirements(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(|line| line.split(" #").next().unwrap_or("").trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('-'))
        .filter(|line| !line.contains("://"))
        .filter_map(|line| {
            let end = line
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                .unwrap_or(line.len());
            let name = &line[..end];
            (!name.is_empty()).then(|| pair("pypi", pypi_name(name)))
        })
        .collect()
}

/// Package names of the `default` and `develop` sections of a Pipfile.lock.
fn pipfile_lock(text: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    ["default", "develop"]
        .iter()
        .filter_map(|section| value[section].as_object())
        .flat_map(|packages| packages.keys())
        .map(|name| pair("pypi", pypi_name(name)))
        .collect()
}

/// Gem names of the `specs:` of the GEM sections of a Gemfile.lock:
/// `    name (version)`, indented by four spaces.
fn gemfile_lock(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut in_gem = false;
    for line in text.lines() {
        if !line.starts_with(' ') {
            in_gem = line.trim() == "GEM";
            continue;
        }
        if in_gem
            && line.starts_with("    ")
            && !line.starts_with("     ")
            && let Some(name) = line.split_whitespace().next()
        {
            found.push(pair("rubygems", name));
        }
    }
    found
}

/// `groupId:artifactId` of the `<dependency>` elements of a pom.xml.
fn pom_xml(text: &str) -> Vec<(String, String)> {
    let Ok(doc) = roxmltree::Document::parse(text) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.tag_name().name() == "dependency")
        .filter_map(|d| {
            let child = |tag: &str| {
                d.children()
                    .find(|c| c.tag_name().name() == tag)
                    .and_then(|c| c.text())
                    .map(str::trim)
            };
            Some(pair(
                "maven",
                format!("{}:{}", child("groupId")?, child("artifactId")?),
            ))
        })
        .collect()
}

/// `group:artifact` of the lines `group:artifact:version=configurations` of a
/// gradle.lockfile.
fn gradle_lockfile(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.starts_with("empty="))
        .filter_map(|line| {
            let coordinates = line.split('=').next()?;
            let mut parts = coordinates.split(':');
            let (group, artifact) = (parts.next()?, parts.next()?);
            parts.next()?;
            Some(pair("maven", format!("{group}:{artifact}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(found: Vec<(String, String)>) -> Vec<String> {
        found.into_iter().map(|(e, n)| format!("{e}:{n}")).collect()
    }

    #[test]
    fn reads_cargo_lock() {
        let text = r#"
version = 4

[[package]]
name = "pelicode-core"
version = "0.1.0"

[[package]]
name = "serde_json"
version = "1.0.140"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        assert_eq!(
            names(cargo_lock(text)),
            ["crates:pelicode-core", "crates:serde_json"]
        );
    }

    #[test]
    fn reads_go_mod_and_sum() {
        let gomod = "module example.com/m\n\ngo 1.22\n\nrequire (\n\tgithub.com/google/uuid v1.6.0\n\tgolang.org/x/mod v0.20.0 // indirect\n)\n\nrequire github.com/BurntSushi/toml v1.4.0\n";
        assert_eq!(
            names(go_mod(gomod)),
            [
                "go:github.com/google/uuid",
                "go:golang.org/x/mod",
                "go:github.com/BurntSushi/toml"
            ]
        );
        let gosum = "github.com/google/uuid v1.6.0 h1:NIvaJ=\ngithub.com/google/uuid v1.6.0/go.mod h1:TIy=\n";
        assert!(
            names(go_sum(gosum))
                .iter()
                .all(|n| n == "go:github.com/google/uuid")
        );
    }

    #[test]
    fn reads_python_files() {
        let reqs = "# comment\nRequests==2.32.3\nnumpy>=2  # inline\ntyping_extensions[x]; python_version < \"3.12\"\n-r other.txt\n-e .\ngit+https://example.org/x.git\n";
        assert_eq!(
            names(requirements(reqs)),
            ["pypi:requests", "pypi:numpy", "pypi:typing-extensions"]
        );
        let poetry = "[[package]]\nname = \"Charset_Normalizer\"\nversion = \"3.4.0\"\n";
        assert_eq!(
            names(toml_packages(poetry, "pypi")),
            ["pypi:charset-normalizer"]
        );
        let pipfile = r#"{"_meta": {}, "default": {"six": {"version": "==1.17.0"}}, "develop": {"PyTest": {}}}"#;
        assert_eq!(names(pipfile_lock(pipfile)), ["pypi:six", "pypi:pytest"]);
    }

    #[test]
    fn reads_gemfile_lock() {
        let text = "GEM\n  remote: https://rubygems.org/\n  specs:\n    rack (3.1.8)\n    thor (1.3.0)\n      rake (>= 12)\n\nPLATFORMS\n  ruby\n\nDEPENDENCIES\n  rack\n";
        assert_eq!(
            names(gemfile_lock(text)),
            ["rubygems:rack", "rubygems:thor"]
        );
    }

    #[test]
    fn reads_maven_and_gradle() {
        let pom = r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><dependencies>
            <dependency><groupId>com.google.code.gson</groupId><artifactId>gson</artifactId><version>2.11.0</version></dependency>
        </dependencies></project>"#;
        assert_eq!(names(pom_xml(pom)), ["maven:com.google.code.gson:gson"]);
        let gradle = "# This is a Gradle generated file\norg.apache.commons:commons-lang3:3.14.0=compileClasspath\nempty=annotationProcessor\n";
        assert_eq!(
            names(gradle_lockfile(gradle)),
            ["maven:org.apache.commons:commons-lang3"]
        );
    }

    #[test]
    fn finds_files_in_a_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("svc/api")).unwrap();
        std::fs::write(dir.path().join("svc/api/requirements-dev.txt"), "six\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "requests\n").unwrap();
        let deps = Dependencies::find(dir.path());
        assert!(deps.contains("pypi", "six"));
        assert_eq!(
            deps.files,
            [dir.path().join("svc/api/requirements-dev.txt")]
        );
    }
}

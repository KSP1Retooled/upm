use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::validate_name;
use crate::tarball::validate_dependencies;

/// Static hosts can't infer a type for extensionless package files. Cloudflare Pages reads this.
const HEADERS: &str = "/*\n  Content-Type: application/json; charset=utf-8\n";
/// Without a 404 page Cloudflare Pages serves the root for unknown paths, instead of a miss.
const NOT_FOUND: &str = "{\"error\":\"not_found\"}\n";

const EPOCH: &str = "1970-01-01T00:00:00.000Z";

#[derive(Debug, Serialize, Deserialize)]
pub struct Packument {
    pub name: String,
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "dist-tags")]
    pub dist_tags: Map<String, Value>,
    pub versions: Map<String, Value>,
    pub time: Map<String, Value>,
}

impl Packument {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            id: name.to_owned(),
            description: None,
            dist_tags: Map::new(),
            versions: Map::new(),
            time: Map::new(),
        }
    }

    pub fn load_or_new(registry: &Path, name: &str) -> Result<Self> {
        let path = registry.join(name);
        if !path.exists() {
            return Ok(Self::new(name));
        }
        let text = std::fs::read_to_string(&path)?;
        serde_json::from_str(&text).with_context(|| format!("failed to parse {path:?}"))
    }

    pub fn save(&self, registry: &Path) -> Result<()> {
        write_if_changed(&registry.join(&self.name), &to_json(self)?)
    }

    pub fn has_tarball(&self, url: &str) -> bool {
        self.versions
            .values()
            .any(|entry| entry["dist"]["tarball"] == url)
    }

    pub fn tarball_of(&self, version: &str) -> Option<&str> {
        self.versions.get(version)?["dist"]["tarball"].as_str()
    }

    pub fn insert(&mut self, version: &Version, entry: Value, published: &str) -> Result<()> {
        self.versions.insert(version.to_string(), entry);
        self.time.insert(version.to_string(), published.into());
        self.normalize()
    }

    /// Everything derived from `versions`, so the output doesn't depend on insertion order.
    fn normalize(&mut self) -> Result<()> {
        let mut versions = self.parsed_versions()?;
        versions.sort();

        self.versions = versions
            .iter()
            .map(|version| {
                let key = version.to_string();
                let entry = self.versions[&key].clone();
                (key, entry)
            })
            .collect();

        let latest = versions
            .iter()
            .rfind(|version| version.pre.is_empty())
            .or(versions.last())
            .context("packument has no versions")?;
        self.dist_tags = Map::from_iter([("latest".to_owned(), latest.to_string().into())]);
        self.description = self.versions[&latest.to_string()]["description"]
            .as_str()
            .map(str::to_owned);

        let mut published: BTreeMap<String, String> = BTreeMap::new();
        for version in self.versions.keys() {
            let time = self.time[version].as_str().unwrap_or(EPOCH);
            published.insert(version.clone(), time.to_owned());
        }
        let oldest = published.values().min().cloned().unwrap_or_default();
        let newest = published.values().max().cloned().unwrap_or_default();

        let mut time = Map::new();
        time.insert("created".to_owned(), oldest.into());
        time.insert("modified".to_owned(), newest.into());
        for version in self.versions.keys() {
            time.insert(version.clone(), published[version].clone().into());
        }
        self.time = time;
        Ok(())
    }

    fn parsed_versions(&self) -> Result<Vec<Version>> {
        self.versions
            .keys()
            .map(|version| {
                Version::parse(version)
                    .with_context(|| format!("{}: invalid version {version:?}", self.name))
            })
            .collect()
    }

    fn latest_entry(&self) -> &Value {
        let latest = self.dist_tags["latest"].as_str().unwrap_or_default();
        &self.versions[latest]
    }

    fn check(&self, registry: &Path) -> Result<()> {
        validate_name(&self.name)?;
        ensure!(self.id == self.name, "{}: _id doesn't match", self.name);
        let file_name = registry.join(&self.name);
        ensure!(file_name.is_file(), "{}: wrong file name", self.name);

        for version in self.parsed_versions()? {
            let entry = &self.versions[&version.to_string()];
            ensure!(
                entry["name"] == self.name.as_str(),
                "{}: wrong name",
                version
            );
            ensure!(
                entry["version"] == version.to_string().as_str(),
                "{}: version {version} has the wrong version field",
                self.name
            );
            validate_dependencies(entry)?;
            let dist = &entry["dist"];
            ensure!(
                dist["tarball"]
                    .as_str()
                    .is_some_and(|url| url.starts_with("https://")),
                "{}: version {version} has no https tarball",
                self.name
            );
            ensure!(
                dist["shasum"].as_str().is_some_and(|sum| sum.len() == 40),
                "{}: version {version} has no shasum",
                self.name
            );
            ensure!(
                dist["integrity"]
                    .as_str()
                    .is_some_and(|sum| sum.starts_with("sha512-")),
                "{}: version {version} has no integrity",
                self.name
            );
            ensure!(
                self.time.contains_key(&version.to_string()),
                "{}: version {version} has no time",
                self.name
            );
        }
        ensure!(
            self.dist_tags.get("latest").is_some_and(|latest| {
                latest
                    .as_str()
                    .is_some_and(|tag| self.versions.contains_key(tag))
            }),
            "{}: latest doesn't name a version",
            self.name
        );
        Ok(())
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)? + "\n")
}

fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, content)?;
    std::fs::rename(&temp, path).with_context(|| format!("failed to write {path:?}"))
}

fn load_all(registry: &Path) -> Result<Vec<Packument>> {
    let mut packuments = Vec::new();
    if !registry.exists() {
        return Ok(packuments);
    }
    for entry in std::fs::read_dir(registry)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_file() && validate_name(&name).is_ok() {
            let text = std::fs::read_to_string(entry.path())?;
            let packument: Packument = serde_json::from_str(&text)
                .with_context(|| format!("failed to parse {:?}", entry.path()))?;
            packuments.push(packument);
        }
    }
    packuments.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packuments)
}

fn index_files(packuments: &[Packument]) -> Vec<(PathBuf, String)> {
    let modified = packuments
        .iter()
        .filter_map(|packument| packument.time.get("modified")?.as_str())
        .max()
        .unwrap_or(EPOCH);

    let mut all = Map::new();
    all.insert("_updated".to_owned(), 99999.into());
    let mut objects = Vec::new();

    for packument in packuments {
        let entry = packument.latest_entry();
        let version = entry["version"].clone();
        let date = packument.time["modified"].clone();
        let description = packument.description.clone().unwrap_or_default();

        let mut summary = json!({
            "name": packument.name,
            "description": description,
            "maintainers": [],
            "versions": { version.as_str().unwrap_or_default(): "latest" },
            "time": { "modified": date },
            "keywords": entry.get("keywords").cloned().unwrap_or(json!([])),
        });
        if let Some(author) = entry.get("author") {
            summary["author"] = author.clone();
        }
        all.insert(packument.name.clone(), summary);

        let mut package = json!({
            "name": packument.name,
            "scope": "",
            "description": description,
            "version": version,
            "keywords": entry.get("keywords").cloned().unwrap_or(json!([])),
            "date": date,
            "publisher": {},
            "maintainers": [],
            "links": { "npm": "" },
        });
        if let Some(display_name) = entry.get("displayName") {
            package["displayName"] = display_name.clone();
        }
        if let Some(author) = entry.get("author") {
            package["author"] = author.clone();
        }
        objects.push(json!({
            "package": package,
            "score": {
                "final": 0,
                "detail": { "quality": 0, "popularity": 0, "maintenance": 0 },
            },
            "searchScore": 1,
        }));
    }

    let search = json!({ "objects": objects, "total": packuments.len(), "time": modified });
    vec![
        (PathBuf::from("-/all"), to_json(&all).unwrap()),
        (PathBuf::from("-/v1/search"), to_json(&search).unwrap()),
        (PathBuf::from("_headers"), HEADERS.to_owned()),
        (PathBuf::from("404.html"), NOT_FOUND.to_owned()),
        (PathBuf::from(".nojekyll"), String::new()),
    ]
}

pub fn write_index(registry: &Path) -> Result<()> {
    let packuments = load_all(registry)?;
    for (path, content) in index_files(&packuments) {
        write_if_changed(&registry.join(path), &content)?;
    }
    Ok(())
}

/// Returns the number of packages that were checked.
pub fn check(registry: &Path) -> Result<usize> {
    let packuments = load_all(registry)?;
    for packument in &packuments {
        packument
            .check(registry)
            .with_context(|| format!("invalid packument {}", packument.name))?;
        if to_json(packument)? != std::fs::read_to_string(registry.join(&packument.name))? {
            bail!("{} isn't in generated form", packument.name);
        }
    }
    for (path, expected) in index_files(&packuments) {
        let actual = std::fs::read_to_string(registry.join(&path)).unwrap_or_default();
        ensure!(
            actual == expected,
            "{path:?} is out of date; run `upm-registry index`"
        );
    }
    Ok(packuments.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str) -> Value {
        json!({
            "name": "com.example.a",
            "version": version,
            "description": format!("v{version}"),
            "dist": {
                "tarball": format!("https://example.com/{version}.tgz"),
                "shasum": "0".repeat(40),
                "integrity": "sha512-AAAA",
            },
        })
    }

    fn packument(versions: &[&str]) -> Packument {
        let mut packument = Packument::new("com.example.a");
        for (day, version) in versions.iter().enumerate() {
            let time = format!("2026-01-{:02}T00:00:00Z", day + 1);
            packument
                .insert(&Version::parse(version).unwrap(), entry(version), &time)
                .unwrap();
        }
        packument
    }

    #[test]
    fn latest_is_the_highest_semver_not_the_newest_insert() {
        let packument = packument(&["1.10.0", "1.9.0"]);
        assert_eq!(packument.dist_tags["latest"], "1.10.0");
        assert_eq!(packument.description.as_deref(), Some("v1.10.0"));
        let keys: Vec<_> = packument.versions.keys().collect();
        assert_eq!(keys, ["1.9.0", "1.10.0"]);
    }

    #[test]
    fn latest_skips_prereleases_unless_there_is_nothing_else() {
        assert_eq!(
            packument(&["1.0.0", "2.0.0-preview.1"]).dist_tags["latest"],
            "1.0.0"
        );
        assert_eq!(
            packument(&["2.0.0-preview.1"]).dist_tags["latest"],
            "2.0.0-preview.1"
        );
    }

    #[test]
    fn time_spans_the_published_versions() {
        let packument = packument(&["1.0.0", "1.1.0"]);
        assert_eq!(packument.time["created"], "2026-01-01T00:00:00Z");
        assert_eq!(packument.time["modified"], "2026-01-02T00:00:00Z");
        assert_eq!(packument.time["1.0.0"], "2026-01-01T00:00:00Z");
    }

    #[test]
    fn finds_existing_tarballs() {
        let packument = packument(&["1.0.0"]);
        assert!(packument.has_tarball("https://example.com/1.0.0.tgz"));
        assert!(!packument.has_tarball("https://example.com/2.0.0.tgz"));
        assert_eq!(
            packument.tarball_of("1.0.0"),
            Some("https://example.com/1.0.0.tgz")
        );
    }

    #[test]
    fn index_lists_the_latest_version_of_each_package() {
        let files = index_files(&[packument(&["1.0.0", "1.1.0"])]);
        let (_, all) = files
            .iter()
            .find(|(path, _)| path.ends_with("all"))
            .unwrap();
        let all: Value = serde_json::from_str(all).unwrap();
        assert_eq!(
            all["com.example.a"]["versions"],
            json!({ "1.1.0": "latest" })
        );

        let (_, search) = files
            .iter()
            .find(|(path, _)| path.ends_with("search"))
            .unwrap();
        let search: Value = serde_json::from_str(search).unwrap();
        assert_eq!(search["total"], 1);
        assert_eq!(search["objects"][0]["package"]["version"], "1.1.0");
    }
}

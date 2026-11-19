use std::io::Read;

use anyhow::{Context, Result, anyhow, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::read::GzDecoder;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha512};

use crate::config::validate_name;

/// Unity rejects a whole package over one npm-style range like `^1.2.0`.
pub fn validate_dependencies(manifest: &Value) -> Result<()> {
    let Some(dependencies) = manifest.get("dependencies").and_then(Value::as_object) else {
        return Ok(());
    };
    for (name, version) in dependencies {
        ensure!(
            version
                .as_str()
                .is_some_and(|version| semver::Version::parse(version).is_ok()),
            "dependency {name} has version {version}, but Unity only accepts exact versions"
        );
    }
    Ok(())
}

pub struct Tarball {
    pub name: String,
    pub version: semver::Version,
    pub manifest: Value,
    dist: Value,
}

impl Tarball {
    pub fn inspect(bytes: &[u8], url: &str) -> Result<Self> {
        let mut manifest = None;
        let mut file_count = 0u64;
        let mut unpacked_size = 0u64;

        let mut archive = tar::Archive::new(GzDecoder::new(bytes));
        for entry in archive.entries().context("not a gzipped tar file")? {
            let mut entry = entry.context("corrupt tar file")?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            file_count += 1;
            unpacked_size += entry.size();

            let path = entry.path()?;
            let path = path.strip_prefix(".").unwrap_or(&path);
            if path == std::path::Path::new("package/package.json") {
                let mut text = String::new();
                entry.read_to_string(&mut text)?;
                manifest = Some(text);
            }
        }

        let text = manifest.ok_or_else(|| anyhow!("tarball has no package/package.json"))?;
        let manifest: Value =
            serde_json::from_str(&text).context("package/package.json is not valid JSON")?;
        let field = |key: &str| {
            manifest[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("package/package.json has no {key}"))
        };

        let name = field("name")?;
        validate_name(&name)?;
        let version = field("version")?;
        let version = semver::Version::parse(&version)
            .with_context(|| format!("version {version:?} is not valid semver"))?;
        validate_dependencies(&manifest)?;

        let dist = json!({
            "tarball": url,
            "shasum": hex::encode(Sha1::digest(bytes)),
            "integrity": format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes))),
            "fileCount": file_count,
            "unpackedSize": unpacked_size,
        });
        Ok(Self {
            name,
            version,
            manifest,
            dist,
        })
    }

    /// The version entry for the packument: `package.json` plus how to fetch it.
    pub fn into_entry(self) -> Value {
        let mut entry = self.manifest;
        entry["_id"] = format!("{}@{}", self.name, self.version).into();
        entry["dist"] = self.dist;
        entry
    }
}

#[cfg(test)]
pub fn make_tarball(path: &str, package_json: &str) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    let mut header = tar::Header::new_gnu();
    header.set_size(package_json.len() as u64);
    header.set_mode(0o644);
    builder
        .append_data(&mut header, path, package_json.as_bytes())
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.com/a.tgz";
    const MANIFEST: &str = r#"{"name":"com.example.a","version":"1.2.3","unity":"2022.3"}"#;

    #[test]
    fn reads_the_manifest_and_hashes_the_archive() {
        let bytes = make_tarball("package/package.json", MANIFEST);
        let tarball = Tarball::inspect(&bytes, URL).unwrap();
        assert_eq!(tarball.name, "com.example.a");
        assert_eq!(tarball.version, semver::Version::new(1, 2, 3));

        let entry = tarball.into_entry();
        assert_eq!(entry["unity"], "2022.3");
        assert_eq!(entry["_id"], "com.example.a@1.2.3");
        assert_eq!(entry["dist"]["tarball"], URL);
        assert_eq!(entry["dist"]["shasum"], hex::encode(Sha1::digest(&bytes)));
        assert_eq!(entry["dist"]["fileCount"], 1);
    }

    #[test]
    fn accepts_a_leading_dot_in_entry_paths() {
        let bytes = make_tarball("./package/package.json", MANIFEST);
        assert!(Tarball::inspect(&bytes, URL).is_ok());
    }

    #[test]
    fn rejects_a_manifest_outside_package_dir() {
        let bytes = make_tarball("mypackage/package.json", MANIFEST);
        assert!(Tarball::inspect(&bytes, URL).is_err());
    }

    #[test]
    fn rejects_a_non_semver_version() {
        let bytes = make_tarball(
            "package/package.json",
            r#"{"name":"com.example.a","version":"1.2"}"#,
        );
        assert!(Tarball::inspect(&bytes, URL).is_err());
    }

    #[test]
    fn rejects_dependency_ranges() {
        let bytes = make_tarball(
            "package/package.json",
            r#"{"name":"com.example.a","version":"1.0.0","dependencies":{"com.example.b":"^1.2.0"}}"#,
        );
        assert!(Tarball::inspect(&bytes, URL).is_err());
    }

    #[test]
    fn accepts_exact_dependencies() {
        let bytes = make_tarball(
            "package/package.json",
            r#"{"name":"com.example.a","version":"1.0.0","dependencies":{"com.example.b":"1.2.0-pre.1"}}"#,
        );
        assert!(Tarball::inspect(&bytes, URL).is_ok());
    }

    #[test]
    fn rejects_a_file_that_is_not_a_tarball() {
        assert!(Tarball::inspect(b"not a tarball", URL).is_err());
    }
}

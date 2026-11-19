use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub repo: String,
    #[serde(default = "default_asset_match")]
    pub asset_match: String,
    #[serde(default)]
    pub prereleases: bool,
}

fn default_asset_match() -> String {
    r"\.tgz$".to_string()
}

impl Package {
    pub fn asset_regex(&self) -> Result<Regex> {
        Regex::new(&self.asset_match).with_context(|| format!("{}: invalid asset_match", self.name))
    }

    fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        ensure!(
            matches!(self.repo.split('/').collect::<Vec<_>>()[..], [owner, repo] if !owner.is_empty() && !repo.is_empty()),
            "{}: repo must be in the format owner/name",
            self.name
        );
        self.asset_regex()?;
        Ok(())
    }
}

/// Package names become file names at the registry root, so this also keeps them from colliding
/// with the registry's own files. Unity doesn't support `@scope` names.
pub fn validate_name(name: &str) -> Result<()> {
    static NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[a-z0-9-]+(\.[a-z0-9_-]+)+$").unwrap());

    ensure!(
        NAME.is_match(name) && name.len() <= 214 && !name.ends_with(".html"),
        "invalid package name {name:?}: expected a reverse-domain name like com.example.package"
    );
    Ok(())
}

pub fn find_all(dir: &Path) -> Result<Vec<Package>> {
    let mut configs = Vec::new();
    collect(dir, &mut configs).with_context(|| format!("failed to load configs from {dir:?}"))?;
    configs.sort_by(|a, b| a.name.cmp(&b.name));

    let mut seen = HashSet::new();
    for config in &configs {
        if !seen.insert(&config.name) {
            bail!("{} is configured more than once", config.name);
        }
    }
    Ok(configs)
}

fn collect(dir: &Path, configs: &mut Vec<Package>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, configs)?;
        } else if path.extension().is_some_and(|ext| ext == "toml") {
            let text = std::fs::read_to_string(&path)?;
            let config: Package =
                toml::from_str(&text).with_context(|| format!("failed to parse {path:?}"))?;
            config
                .validate()
                .with_context(|| format!("invalid config {path:?}"))?;
            configs.push(config);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_reverse_domain_names() {
        validate_name("com.ksp1retooled.content-import").unwrap();
        validate_name("com.unity.xr.arcore").unwrap();
    }

    #[test]
    fn rejects_names_that_are_not_safe_registry_paths() {
        for name in [
            "@scope/package",
            "Com.Example.Package",
            "nodots",
            "../etc.passwd",
            "com.example/package",
            "-",
            "_headers",
            ".nojekyll",
            "com.example.html",
        ] {
            assert!(validate_name(name).is_err(), "{name} should be rejected");
        }
    }

    #[test]
    fn example_config_is_valid() {
        let text = include_str!("../examples/com.example.mypackage.toml");
        let config: Package = toml::from_str(text).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn rejects_unknown_fields() {
        let text = "name = \"com.example.a\"\nrepo = \"a/b\"\nasset = \"x\"\n";
        assert!(toml::from_str::<Package>(text).is_err());
    }
}

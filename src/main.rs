use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};

use crate::config::Package;
use crate::github::{Asset, GitHub, Release};
use crate::registry::Packument;
use crate::tarball::Tarball;

mod config;
mod github;
mod registry;
mod tarball;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct App {
    #[clap(subcommand)]
    cmd: Cmds,

    /// GitHub token to use for API requests
    #[clap(
        short,
        long,
        global = true,
        env = "GITHUB_TOKEN",
        hide_env_values = true
    )]
    github_token: Option<String>,

    /// Where to find the package configuration files
    #[clap(short, long, global = true, default_value = "./configs")]
    configs_dir: PathBuf,

    /// Where to write the generated registry
    #[clap(short, long, global = true, default_value = "./registry")]
    out_dir: PathBuf,
}

#[derive(Subcommand)]
enum Cmds {
    /// Add new releases to the registry
    Generate {
        /// The packages to update, or all of them if empty
        #[clap(value_delimiter = ',')]
        filter: Vec<String>,

        /// Only update packages published from this owner/name repo
        #[clap(long)]
        repo: Option<String>,

        /// Use the release with this tag instead of the latest one
        #[clap(long, conflicts_with = "all")]
        tag: Option<String>,

        /// Use every release instead of the latest one
        #[clap(long)]
        all: bool,

        /// Replace a version even if it's already published from another tarball
        #[clap(long)]
        force: bool,
    },
    /// Rebuild the search index and host files from the existing packages
    Index,
    /// Check that the configs and the generated registry are valid and up to date
    Check,
}

#[derive(Clone, Copy)]
enum Selection<'a> {
    Latest,
    Tag(&'a str),
    All,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let app = App::parse();
    match app.cmd {
        Cmds::Generate {
            filter,
            repo,
            tag,
            all,
            force,
        } => {
            let selection = match (&tag, all) {
                (Some(tag), _) => Selection::Tag(tag),
                (None, true) => Selection::All,
                (None, false) => Selection::Latest,
            };
            let configs = select(
                config::find_all(&app.configs_dir)?,
                &filter,
                repo.as_deref(),
            )?;
            let gh = GitHub::new(app.github_token)?;
            generate(&gh, &configs, &app.out_dir, selection, force)
        }
        Cmds::Index => registry::write_index(&app.out_dir),
        Cmds::Check => {
            let configs = config::find_all(&app.configs_dir)?;
            let packages = registry::check(&app.out_dir)?;
            println!("{} configs and {packages} packages ok", configs.len());
            Ok(())
        }
    }
}

fn select(configs: Vec<Package>, filter: &[String], repo: Option<&str>) -> Result<Vec<Package>> {
    if let Some(unknown) = filter
        .iter()
        .find(|name| !configs.iter().any(|config| config.name == **name))
    {
        bail!("no config for package {unknown}");
    }
    let selected: Vec<_> = configs
        .into_iter()
        .filter(|config| filter.is_empty() || filter.contains(&config.name))
        .filter(|config| repo.is_none_or(|repo| config.repo.eq_ignore_ascii_case(repo)))
        .collect();
    if selected.is_empty() {
        bail!("no configs match: nothing to update");
    }
    Ok(selected)
}

fn generate(
    gh: &GitHub,
    configs: &[Package],
    registry: &Path,
    selection: Selection,
    force: bool,
) -> Result<()> {
    let mut failed = 0;
    let mut matched = 0;
    for config in configs {
        match update_package(gh, config, registry, selection, force) {
            Ok(count) => matched += count,
            Err(error) => {
                eprintln!("error: {}: {error:#}", config.name);
                failed += 1;
            }
        }
    }
    registry::write_index(registry)?;

    if failed > 0 {
        bail!("{failed} of {} packages failed", configs.len());
    }
    // Packages sharing a repo don't each have an asset in every release, but a run that found
    // nothing at all means the release or the config is wrong.
    if matched == 0 && !matches!(selection, Selection::All) {
        bail!("no release asset matched any selected package");
    }
    Ok(())
}

fn wanted_releases(gh: &GitHub, config: &Package, selection: Selection) -> Result<Vec<Release>> {
    let releases = match selection {
        Selection::Tag(tag) => vec![gh.release_by_tag(&config.repo, tag)?],
        Selection::Latest if !config.prereleases => vec![gh.latest_release(&config.repo)?],
        Selection::Latest => {
            let releases = gh.releases(&config.repo)?;
            let newest = releases.into_iter().find(|release| !release.draft);
            vec![newest.ok_or_else(|| anyhow!("{} has no releases", config.repo))?]
        }
        Selection::All => gh.releases(&config.repo)?,
    };
    let mut releases: Vec<_> = releases
        .into_iter()
        .filter(|release| !release.draft && (config.prereleases || !release.prerelease))
        .collect();
    releases.reverse();
    Ok(releases)
}

fn matching_asset<'a>(config: &Package, release: &'a Release) -> Result<Option<&'a Asset>> {
    let regex = config.asset_regex()?;
    let mut matches = release
        .assets
        .iter()
        .filter(|asset| regex.is_match(&asset.name));
    match (matches.next(), matches.next()) {
        (Some(a), Some(b)) => bail!(
            "asset_match is ambiguous in {}: {} and {}",
            release.tag_name,
            a.name,
            b.name
        ),
        (asset, _) => Ok(asset),
    }
}

/// Returns how many release assets were found for the package, published or not.
fn update_package(
    gh: &GitHub,
    config: &Package,
    registry: &Path,
    selection: Selection,
    force: bool,
) -> Result<usize> {
    let mut packument = Packument::load_or_new(registry, &config.name)?;
    let mut matched = 0;
    let mut changed = false;

    for release in wanted_releases(gh, config, selection)? {
        let Some(asset) = matching_asset(config, &release)? else {
            println!("{}: no asset in {}", config.name, release.tag_name);
            continue;
        };
        matched += 1;
        if packument.has_tarball(&asset.browser_download_url) && !force {
            println!("{}: {} already published", config.name, release.tag_name);
            continue;
        }

        let url = &asset.browser_download_url;
        let bytes = gh.download(url)?;
        let tarball = Tarball::inspect(&bytes, url)
            .with_context(|| format!("{} is not a valid package", asset.name))?;
        if tarball.name != config.name {
            bail!("{} contains package {}", asset.name, tarball.name);
        }

        let version = tarball.version.clone();
        if let Some(existing) = packument.tarball_of(&version.to_string())
            && !force
        {
            bail!("version {version} is already published from {existing}");
        }

        let published = release
            .published_at
            .as_deref()
            .with_context(|| format!("release {} has no publish date", release.tag_name))?;
        packument.insert(&version, tarball.into_entry(), published)?;
        changed = true;
        println!("{}: added {version} from {}", config.name, release.tag_name);
    }

    if changed {
        packument.save(registry)?;
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, repo: &str) -> Package {
        toml::from_str(&format!("name = \"{name}\"\nrepo = \"{repo}\"\n")).unwrap()
    }

    fn configs() -> Vec<Package> {
        vec![
            package("com.example.a", "Owner/One"),
            package("com.example.b", "Owner/Two"),
        ]
    }

    #[test]
    fn select_matches_repos_case_insensitively() {
        let selected = select(configs(), &[], Some("owner/two")).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "com.example.b");
    }

    #[test]
    fn select_fails_for_an_unconfigured_repo() {
        assert!(select(configs(), &[], Some("someone/else")).is_err());
    }

    #[test]
    fn select_fails_for_an_unknown_package() {
        assert!(select(configs(), &["com.example.c".to_owned()], None).is_err());
    }

    fn release(assets: &[&str]) -> Release {
        Release {
            tag_name: "v1".to_owned(),
            prerelease: false,
            draft: false,
            published_at: None,
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: (*name).to_owned(),
                    browser_download_url: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn asset_match_picks_the_only_tarball() {
        let config = package("com.example.a", "Owner/One");
        let release = release(&["mod.zip", "a-1.0.0.tgz"]);
        let asset = matching_asset(&config, &release).unwrap().unwrap();
        assert_eq!(asset.name, "a-1.0.0.tgz");
    }

    #[test]
    fn asset_match_rejects_ambiguous_releases() {
        let config = package("com.example.a", "Owner/One");
        assert!(matching_asset(&config, &release(&["a.tgz", "b.tgz"])).is_err());
    }

    #[test]
    fn asset_match_allows_releases_without_a_package() {
        let config = package("com.example.a", "Owner/One");
        assert!(
            matching_asset(&config, &release(&["a.zip"]))
                .unwrap()
                .is_none()
        );
    }
}

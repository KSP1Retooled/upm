use anyhow::{Context, Result};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const API: &str = "https://api.github.com";
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub prerelease: bool,
    pub draft: bool,
    pub published_at: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

pub struct GitHub {
    client: Client,
    token: Option<String>,
}

impl GitHub {
    pub fn new(token: Option<String>) -> Result<Self> {
        let client = Client::builder().user_agent(USER_AGENT).build()?;
        Ok(Self { client, token })
    }

    fn api(&self, url: &str) -> RequestBuilder {
        let request = self
            .client
            .get(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        match &self.token {
            Some(token) => request.header(AUTHORIZATION, format!("Bearer {token}")),
            None => request,
        }
    }

    fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.api(url)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json())
            .with_context(|| format!("GET {url} failed"))
    }

    pub fn latest_release(&self, repo: &str) -> Result<Release> {
        self.get(&format!("{API}/repos/{repo}/releases/latest"))
    }

    pub fn release_by_tag(&self, repo: &str, tag: &str) -> Result<Release> {
        self.get(&format!("{API}/repos/{repo}/releases/tags/{tag}"))
    }

    /// Every release of `repo`, newest first.
    pub fn releases(&self, repo: &str) -> Result<Vec<Release>> {
        let mut releases = Vec::new();
        for page in 1.. {
            let batch: Vec<Release> = self.get(&format!(
                "{API}/repos/{repo}/releases?per_page=100&page={page}"
            ))?;
            let done = batch.len() < 100;
            releases.extend(batch);
            if done {
                break;
            }
        }
        Ok(releases)
    }

    /// The token is left off on purpose: the download redirects to another host.
    pub fn download(&self, url: &str) -> Result<Vec<u8>> {
        let bytes = self
            .client
            .get(url)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.bytes())
            .with_context(|| format!("failed to download {url}"))?;
        Ok(bytes.to_vec())
    }
}

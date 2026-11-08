use crate::{
    config::{Config, validate_base_url},
    engine::{self, Action, Job, Status},
};
use anyhow::{Context, Result, bail};
use reqwest::blocking::Client as Http;
use serde::de::DeserializeOwned;
use std::{path::PathBuf, time::Duration};

#[derive(Clone)]
pub struct Client {
    http: Http,
    base: String,
    token: String,
}
impl Client {
    pub fn new(config: &Config, url: Option<String>, token_file: Option<PathBuf>) -> Result<Self> {
        config.validate()?;
        let base = url
            .unwrap_or_else(|| config.base_url())
            .trim_end_matches('/')
            .to_string();
        // A non-loopback endpoint must be explicitly selected in the configuration.
        // URL overrides keep the loopback-only policy to avoid leaking a token accidentally.
        if base != config.base_url() {
            validate_base_url(&base)?;
        }
        let token = if let Some(file) = token_file {
            std::fs::read_to_string(file)?.trim().to_owned()
        } else {
            engine::read_token(config)?
        };
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Invalid token file");
        }
        Ok(Self {
            http: Http::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()?,
            base,
            token,
        })
    }
    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .context("Cannot reach yonix. Start the service or check the SSH tunnel.")?
            .error_for_status()?
            .json()
            .map_err(Into::into)
    }
    pub fn status(&self) -> Result<Status> {
        self.get("/api/status")
    }
    pub fn jobs(&self) -> Result<Vec<Job>> {
        self.get("/api/jobs")
    }
    pub fn job(&self, id: &str) -> Result<Job> {
        self.get(&format!("/api/jobs/{id}"))
    }
    pub fn log(&self, id: &str) -> Result<String> {
        Ok(self
            .http
            .get(format!("{}/api/jobs/{id}/log", self.base))
            .bearer_auth(&self.token)
            .send()?
            .error_for_status()?
            .text()?)
    }
    pub fn start(&self, action: Action, confirmed: bool) -> Result<Job> {
        let response = self
            .http
            .post(format!("{}/api/jobs", self.base))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"action": action, "confirmed": confirmed}))
            .send()?;
        if !response.status().is_success() {
            bail!("{}", response.text()?);
        }
        Ok(response.json()?)
    }
}

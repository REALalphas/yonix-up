use crate::{
    config::{Config, validate_base_url},
    engine::{self, Action, Job, Status},
    peers::{Connection, Host},
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
        let client = Self::endpoint(config, url)?;
        let token = if let Some(file) = token_file {
            std::fs::read_to_string(file)?.trim().to_owned()
        } else {
            engine::read_token(config)?
        };
        client.with_token(&token)
    }

    pub fn endpoint(config: &Config, url: Option<String>) -> Result<Self> {
        config.validate()?;
        let base = url
            .unwrap_or_else(|| config.base_url())
            .trim_end_matches('/')
            .to_string();
        if base != config.base_url() {
            validate_base_url(&base)?;
        }
        Ok(Self {
            http: Http::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()?,
            base,
            token: String::new(),
        })
    }

    pub fn with_token(&self, token: &str) -> Result<Self> {
        let token = token.trim();
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Invalid access token");
        }
        let mut client = self.clone();
        client.token = token.to_owned();
        Ok(client)
    }

    #[cfg(feature = "gui")]
    pub fn has_token(&self) -> bool {
        !self.token.is_empty()
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
    pub fn post_json(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .context("Cannot reach yonix. Start the service or check the SSH tunnel.")?;
        if !response.status().is_success() {
            bail!("{}", response.text()?);
        }
        Ok(response.json()?)
    }
    pub fn post_no_auth(
        base: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let http = Http::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()?;
        let response = http
            .post(format!("{}{path}", base.trim_end_matches('/')))
            .json(body)
            .send()
            .context("Cannot reach the pairing host. Check its address and service.")?;
        if !response.status().is_success() {
            bail!("{}", response.text()?);
        }
        Ok(response.json()?)
    }
    pub fn status(&self) -> Result<Status> {
        self.get("/api/status")
    }

    pub fn hosts(&self) -> Result<Vec<Host>> {
        self.get("/api/hosts")
    }

    pub fn status_for(&self, host: &str) -> Result<Status> {
        if host == "local" {
            self.status()
        } else {
            self.get(&format!("/api/hosts/{host}/status"))
        }
    }

    pub fn jobs_for(&self, host: &str) -> Result<Vec<Job>> {
        if host == "local" {
            self.jobs()
        } else {
            self.get(&format!("/api/hosts/{host}/jobs"))
        }
    }
    pub fn jobs(&self) -> Result<Vec<Job>> {
        self.get("/api/jobs")
    }
    pub fn job(&self, id: &str) -> Result<Job> {
        self.get(&format!("/api/jobs/{id}"))
    }

    pub fn job_for(&self, host: &str, id: &str) -> Result<Job> {
        if host == "local" {
            self.job(id)
        } else {
            self.get(&format!("/api/hosts/{host}/jobs/{id}"))
        }
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

    pub fn log_for(&self, host: &str, id: &str) -> Result<String> {
        if host == "local" {
            self.log(id)
        } else {
            self.http
                .get(format!("{}/api/hosts/{host}/jobs/{id}/log", self.base))
                .bearer_auth(&self.token)
                .send()?
                .error_for_status()?
                .text()
                .map_err(Into::into)
        }
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

    pub fn start_for(&self, host: &str, action: Action, confirmed: bool) -> Result<Job> {
        if host == "local" {
            self.start(action, confirmed)
        } else {
            let response = self
                .http
                .post(format!("{}/api/hosts/{host}/jobs", self.base))
                .bearer_auth(&self.token)
                .json(&serde_json::json!({"action": action, "confirmed": confirmed}))
                .send()?;
            if !response.status().is_success() {
                bail!("{}", response.text()?);
            }
            Ok(response.json()?)
        }
    }

    pub fn connections(&self) -> Result<Vec<Connection>> {
        self.get("/api/connections")
    }

    pub fn post_api(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        self.post_json(&format!("/api/{path}"), body)
    }

    pub fn get_api(&self, path: &str) -> Result<serde_json::Value> {
        self.get(&format!("/api/{path}"))
    }
}

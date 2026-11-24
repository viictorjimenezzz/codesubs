//! API-key accounts: a stored key plus the models an agent should use.
//!
//! DETAILS >>>
//! Health checks make one-token requests, so they cost almost nothing but do
//! exercise the real route Claude Code or Codex will use.

use std::{collections::BTreeMap, time::Duration};

use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AddGatewayOptions, Codesubs, Error, GatewayExecution, GatewayHealth, GatewayRecord,
    GatewayStatus, Provider, Result, gateway_preset,
    platform::keychain::{self, gateway_service},
    storage::validate_slug,
};

const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const KEY_VARIABLE: &str = "CODESUBS_GATEWAY_KEY";

impl Codesubs {
    pub async fn list_gateways(&self) -> Result<Vec<GatewayRecord>> {
        self.repository.gateways()
    }

    pub async fn resolve_gateway(&self, reference: &str) -> Result<GatewayRecord> {
        let needle = reference.to_lowercase();
        let gateways = self.list_gateways().await?;
        let matches: Vec<_> = match gateways.iter().find(|g| g.reference() == needle) {
            Some(exact) => vec![exact],
            None => gateways
                .iter()
                .filter(|g| g.slug == needle || g.service == needle)
                .collect(),
        };
        match matches.as_slice() {
            [] => Err(Error::AccountNotFound(reference.into())),
            [one] => Ok((*one).clone()),
            many => Err(Error::AmbiguousReference {
                reference: reference.into(),
                matches: many
                    .iter()
                    .map(|g| g.reference())
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
        }
    }

    /// Saves a key and its model choice after proving the key works.
    pub async fn add_gateway(
        &self,
        service: &str,
        key: &str,
        options: AddGatewayOptions,
    ) -> Result<GatewayRecord> {
        let key = key.trim();
        let preset = gateway_preset(service);
        let slug = options.slug.unwrap_or_else(|| "default".into());
        validate_slug(service)?;
        validate_slug(&slug)?;
        if Provider::ALL.iter().any(|p| p.as_str() == service) {
            return Err(Error::InvalidInput(format!(
                "{service:?} is a subscription provider; name the API service instead"
            )));
        }
        let pick =
            |explicit: Option<String>, preset: Option<&str>| explicit.or(preset.map(str::to_owned));
        let anthropic_base_url = pick(
            options.anthropic_base_url,
            preset.as_ref().map(|p| p.anthropic_base_url),
        );
        let openai_base_url = pick(
            options.openai_base_url,
            preset.as_ref().map(|p| p.openai_base_url),
        );
        let (Some(big_model), Some(small_model)) = (
            pick(options.big_model, preset.as_ref().map(|p| p.big_model)),
            pick(options.small_model, preset.as_ref().map(|p| p.small_model)),
        ) else {
            return Err(Error::InvalidInput(
                "--big and --small are required for a service without a preset".into(),
            ));
        };
        if anthropic_base_url.is_none() && openai_base_url.is_none() {
            return Err(Error::InvalidInput(
                "give --anthropic-url and/or --openai-url for a service without a preset".into(),
            ));
        }
        let mut gateway = GatewayRecord {
            id: Uuid::new_v4(),
            service: service.into(),
            slug,
            anthropic_base_url,
            openai_base_url,
            big_model,
            small_model,
            health: None,
        };
        if !options.skip_check {
            let health = check_gateway(&gateway, key).await;
            if health.status == GatewayStatus::Down {
                return Err(Error::Provider(format!(
                    "{} is not usable: {}",
                    gateway.reference(),
                    health.message.unwrap_or_default()
                )));
            }
            gateway.health = Some(health);
        }
        keychain::store_secret(&gateway_service(&gateway.reference()), key).await?;
        let saved = gateway.clone();
        let stored = self.repository.update_gateways(|gateways| {
            if gateways.iter().any(|g| g.reference() == saved.reference()) {
                return Err(Error::SlugTaken(saved.reference()));
            }
            gateways.push(saved);
            Ok(())
        });
        if let Err(error) = stored {
            // Do not orphan the key of a record that was never saved. A
            // duplicate keeps the existing record's key untouched.
            if !matches!(error, Error::SlugTaken(_)) {
                keychain::remove_secret(&gateway_service(&gateway.reference())).await?;
            }
            return Err(error);
        }
        Ok(gateway)
    }

    pub async fn set_gateway_models(
        &self,
        reference: &str,
        big: Option<String>,
        small: Option<String>,
    ) -> Result<GatewayRecord> {
        let id = self.resolve_gateway(reference).await?.id;
        self.repository.update_gateways(|gateways| {
            let gateway = gateways
                .iter_mut()
                .find(|g| g.id == id)
                .ok_or_else(|| Error::StaleState(reference.into()))?;
            gateway.big_model = big.unwrap_or_else(|| gateway.big_model.clone());
            gateway.small_model = small.unwrap_or_else(|| gateway.small_model.clone());
            gateway.health = None;
            Ok(gateway.clone())
        })
    }

    pub async fn remove_gateway(&self, reference: &str) -> Result<GatewayRecord> {
        let gateway = self.resolve_gateway(reference).await?;
        self.repository.update_gateways(|gateways| {
            gateways.retain(|g| g.id != gateway.id);
            Ok(())
        })?;
        keychain::remove_secret(&gateway_service(&gateway.reference())).await?;
        let folder = self.repository.gateway_home(&gateway, Provider::Claude);
        crate::storage::remove_directory(folder.parent().expect("gateway home has a parent"))?;
        Ok(gateway)
    }

    /// Checks one gateway or all of them and records the outcomes.
    pub async fn check_gateways(&self, reference: Option<&str>) -> Result<Vec<GatewayRecord>> {
        let targets = match reference {
            Some(reference) => vec![self.resolve_gateway(reference).await?],
            None => self.list_gateways().await?,
        };
        let mut tasks = tokio::task::JoinSet::new();
        for (index, gateway) in targets.iter().cloned().enumerate() {
            tasks.spawn(async move {
                let health = match gateway_key(&gateway).await {
                    Ok(key) => check_gateway(&gateway, &key).await,
                    Err(error) => GatewayHealth {
                        status: GatewayStatus::Down,
                        checked_at: Utc::now(),
                        latency_ms: 0,
                        message: Some(error.to_string()),
                    },
                };
                (index, health)
            });
        }
        let mut results = Vec::with_capacity(targets.len());
        while let Some(task) = tasks.join_next().await {
            results.push(task.map_err(super::accounts::interrupted)?);
        }
        results.sort_by_key(|(index, _)| *index);
        self.repository.update_gateways(|gateways| {
            for (target, (_, health)) in targets.iter().zip(results) {
                if let Some(gateway) = gateways.iter_mut().find(|g| g.id == target.id) {
                    gateway.health = Some(health);
                }
            }
            Ok(())
        })?;
        let ids: Vec<_> = targets.iter().map(|g| g.id).collect();
        Ok(self
            .list_gateways()
            .await?
            .into_iter()
            .filter(|g| ids.contains(&g.id))
            .collect())
    }

    /// The environment that runs `agent` against a gateway. Prepares the
    /// agent's private configuration folder and reads the key from the Keychain.
    pub async fn gateway_env(&self, reference: &str, agent: Provider) -> Result<GatewayExecution> {
        let gateway = self.resolve_gateway(reference).await?;
        let key = gateway_key(&gateway).await?;
        let home = self.repository.gateway_home(&gateway, agent);
        self.repository.prepare_gateway_home(&home, agent)?;
        let url = |url: &Option<String>, flag: &str| {
            url.clone().ok_or_else(|| {
                Error::Unsupported(format!(
                    "{} has no {flag} endpoint for {agent}",
                    gateway.reference()
                ))
            })
        };
        let mut set = BTreeMap::new();
        let mut unset: Vec<String> = agent
            .conflicting_environment_variables()
            .iter()
            .map(|name| (*name).into())
            .collect();
        match agent {
            Provider::Claude => {
                let (big, small) = (&gateway.big_model, &gateway.small_model);
                for (name, value) in [
                    (
                        "ANTHROPIC_BASE_URL",
                        url(&gateway.anthropic_base_url, "Anthropic")?,
                    ),
                    ("ANTHROPIC_AUTH_TOKEN", key),
                    ("ANTHROPIC_MODEL", big.clone()),
                    ("ANTHROPIC_DEFAULT_OPUS_MODEL", big.clone()),
                    ("ANTHROPIC_DEFAULT_SONNET_MODEL", big.clone()),
                    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", small.clone()),
                    ("ANTHROPIC_SMALL_FAST_MODEL", small.clone()),
                    ("DISABLE_TELEMETRY", "1".into()),
                    ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
                ] {
                    set.insert(name.into(), value);
                }
                unset.retain(|name| !set.contains_key(name));
            }
            Provider::Codex => {
                let base_url = url(&gateway.openai_base_url, "OpenAI")?;
                crate::storage::write_text(
                    &home.join("config.toml"),
                    &codex_config(&gateway, &base_url),
                )?;
                set.insert(KEY_VARIABLE.into(), key);
                unset.push("OPENAI_API_KEY".into());
            }
        }
        set.insert(
            agent.home_environment_variable().into(),
            home.to_string_lossy().into(),
        );
        Ok(GatewayExecution {
            gateway,
            home,
            set,
            unset,
        })
    }
}

async fn gateway_key(gateway: &GatewayRecord) -> Result<String> {
    keychain::read_secret(&gateway_service(&gateway.reference()))
        .await?
        .ok_or_else(|| {
            Error::Provider(format!(
                "the API key for {} is missing from the Keychain; add it again",
                gateway.reference()
            ))
        })
}

fn codex_config(gateway: &GatewayRecord, base_url: &str) -> String {
    format!(
        "# Generated by codesubs for {reference}. Edits are overwritten.\n\
         model = \"{model}\"\n\
         model_provider = \"{service}\"\n\n\
         [model_providers.{service}]\n\
         name = \"{service}\"\n\
         base_url = \"{base_url}\"\n\
         env_key = \"{KEY_VARIABLE}\"\n\
         wire_api = \"responses\"\n",
        reference = gateway.reference(),
        model = gateway.big_model,
        service = gateway.service,
    )
}

/// Proves the key is accepted and that each configured model answers a
/// one-token request on the route its agent uses.
async fn check_gateway(gateway: &GatewayRecord, key: &str) -> GatewayHealth {
    let started = std::time::Instant::now();
    let client = match reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => return health(started, GatewayStatus::Down, Some(error.to_string())),
    };
    let (catalog_url, completion_url) =
        match (&gateway.anthropic_base_url, &gateway.openai_base_url) {
            (Some(root), _) => (format!("{root}/v1/models"), format!("{root}/v1/messages")),
            (None, Some(base)) => (format!("{base}/models"), format!("{base}/chat/completions")),
            (None, None) => {
                return health(started, GatewayStatus::Down, Some("no endpoint".into()));
            }
        };
    let anthropic = gateway.anthropic_base_url.is_some();
    let catalog = match client.get(&catalog_url).bearer_auth(key).send().await {
        Ok(response) if response.status().is_success() => {
            response.json::<Value>().await.ok().map(|body| {
                body["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|model| model["id"].as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
        }
        Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
            return health(
                started,
                GatewayStatus::Down,
                Some("The API key was rejected.".into()),
            );
        }
        // Some services do not list models; the completion probes decide.
        Ok(_) => None,
        Err(error) => {
            let reason = if error.is_timeout() {
                "timed out"
            } else {
                "unreachable"
            };
            return health(
                started,
                GatewayStatus::Down,
                Some(format!("The service is {reason}.")),
            );
        }
    };

    let (big, small) = (gateway.big_model.as_str(), gateway.small_model.as_str());
    let probe = |model| {
        probe_model(
            &client,
            &completion_url,
            key,
            model,
            anthropic,
            catalog.as_deref(),
        )
    };
    let probes = if big == small {
        vec![probe(big).await]
    } else {
        let (big, small) = tokio::join!(probe(big), probe(small));
        vec![big, small]
    };
    let total = probes.len();
    let failures: Vec<_> = probes.into_iter().filter_map(|probe| probe.err()).collect();
    let status = match failures.len() {
        0 => GatewayStatus::Ok,
        count if count < total => GatewayStatus::Degraded,
        _ => GatewayStatus::Down,
    };
    health(
        started,
        status,
        (!failures.is_empty()).then(|| failures.join("; ")),
    )
}

fn health(
    started: std::time::Instant,
    status: GatewayStatus,
    message: Option<String>,
) -> GatewayHealth {
    GatewayHealth {
        status,
        checked_at: Utc::now(),
        latency_ms: started.elapsed().as_millis() as u64,
        message,
    }
}

async fn probe_model(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    model: &str,
    anthropic: bool,
    catalog: Option<&[String]>,
) -> std::result::Result<(), String> {
    if catalog.is_some_and(|ids| !ids.iter().any(|id| id == model)) {
        return Err(format!("{model} is not in the service's model list"));
    }
    let body =
        json!({"model": model, "max_tokens": 1, "messages": [{"role": "user", "content": "ping"}]});
    let mut request = client.post(url).bearer_auth(key).json(&body);
    if anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    let response = request
        .send()
        .await
        .map_err(|_| format!("{model} did not answer in time"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status.as_u16() == 429 {
        return Err(format!("{model} is rate limited right now"));
    }
    let detail = response
        .json::<Value>()
        .await
        .ok()
        .and_then(|body| {
            body["error"]["message"]
                .as_str()
                .map(|text| text.chars().take(120).collect::<String>())
        })
        .map(|text| format!(": {text}"))
        .unwrap_or_default();
    Err(format!("{model} returned HTTP {}{detail}", status.as_u16()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves canned responses: `/v1/models` lists `good` and `busy`; one-token
    /// completions answer 200 for `good`, 429 for `busy`, 503 otherwise. A
    /// wrong bearer token is rejected with 401.
    async fn serve() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buffer = vec![0; 8192];
                    let read = socket.read(&mut buffer).await.unwrap();
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let (status, body) = if !request.contains("Bearer sk-good") {
                        (401, "{}".to_owned())
                    } else if request.starts_with("GET /v1/models") {
                        (200, r#"{"data":[{"id":"good"},{"id":"busy"}]}"#.into())
                    } else if request.contains(r#""model":"good""#) {
                        (200, "{}".into())
                    } else if request.contains(r#""model":"busy""#) {
                        (429, "{}".into())
                    } else {
                        (503, r#"{"error":{"message":"No providers"}}"#.into())
                    };
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        format!("http://{address}")
    }

    fn gateway(root: String, big: &str, small: &str) -> GatewayRecord {
        GatewayRecord {
            id: Uuid::new_v4(),
            service: "test".into(),
            slug: "default".into(),
            anthropic_base_url: Some(root),
            openai_base_url: None,
            big_model: big.into(),
            small_model: small.into(),
            health: None,
        }
    }

    #[tokio::test]
    async fn health_reflects_key_and_each_model() {
        let root = serve().await;
        let status = |big: &'static str, small: &'static str, key: &'static str| {
            let gateway = gateway(root.clone(), big, small);
            async move { check_gateway(&gateway, key).await }
        };
        assert_eq!(
            status("good", "good", "sk-good").await.status,
            GatewayStatus::Ok
        );
        let busy = status("good", "busy", "sk-good").await;
        assert_eq!(busy.status, GatewayStatus::Degraded);
        assert!(busy.message.unwrap().contains("rate limited"));
        let missing = status("good", "nope", "sk-good").await;
        assert_eq!(missing.status, GatewayStatus::Degraded);
        assert!(
            missing
                .message
                .unwrap()
                .contains("not in the service's model list")
        );
        assert_eq!(
            status("nope", "nope", "sk-good").await.status,
            GatewayStatus::Down
        );
        let rejected = status("good", "good", "sk-bad").await;
        assert_eq!(rejected.status, GatewayStatus::Down);
        assert!(rejected.message.unwrap().contains("rejected"));
    }

    #[test]
    fn codex_config_points_at_the_service_with_the_big_model() {
        let mut record = gateway("https://example.com".into(), "big", "small");
        record.openai_base_url = Some("https://example.com/v1".into());
        let config = codex_config(&record, "https://example.com/v1");
        assert!(config.contains("model = \"big\""));
        assert!(config.contains("model_provider = \"test\""));
        assert!(config.contains("base_url = \"https://example.com/v1\""));
        assert!(config.contains("wire_api = \"responses\""));
        assert!(!config.contains("sk-"));
    }
}

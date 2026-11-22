//! User-configured compatible endpoints. One explicit API dialect per route.

use dr_agent::{CompatibleProtocol, LocalRoute, ModelErrorCategory};
use reqwest::Url;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::local::{DiscoverError, Discovery, LocalModel};
use crate::{ProviderError, Result};

/// The resource prefix, including any user-supplied path (e.g. /api/v1).
/// TLS is required outside loopback.
pub fn base_url(raw: &str) -> Result<String> {
    let url = Url::parse(raw.trim()).map_err(|_| invalid())?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn invalid() -> ProviderError {
    ProviderError::new(ModelErrorCategory::Unsupported, false)
}

pub fn endpoint(base: &str, resource: &str) -> Result<String> {
    Ok(format!("{}/{resource}", base_url(base)?))
}

/// A stored key is bound to the frozen route's id, URL and dialect.
pub fn credential_scope(route: &LocalRoute) -> Result<String> {
    crate::credentials::local_key_id(&route.server)?;
    let Some(protocol) = route.protocol else {
        return Ok(route.server.clone());
    };
    let mut hash = Sha256::new();
    hash.update(b"drifting-gateway-v1\0");
    hash.update(route.server.as_bytes());
    hash.update(b"\0");
    hash.update(protocol.resource());
    hash.update(b"\0");
    hash.update(base_url(&route.base_url)?);
    let digest = format!("{:x}", hash.finalize());
    Ok(format!("gw-{}", &digest[..60]))
}

pub(crate) fn authenticate(
    request: reqwest::RequestBuilder,
    protocol: CompatibleProtocol,
    key: Option<&str>,
) -> reqwest::RequestBuilder {
    if protocol == CompatibleProtocol::AnthropicMessages {
        let request = request.header("anthropic-version", "2023-06-01");
        match key {
            Some(key) => request.header("x-api-key", key),
            None => request,
        }
    } else {
        match key {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }
}

pub async fn discover(
    client: &reqwest::Client,
    base: &str,
    protocol: CompatibleProtocol,
    key: Option<&str>,
) -> std::result::Result<Discovery, DiscoverError> {
    let url = endpoint(base, "models").map_err(|_| DiscoverError::Unreachable)?;
    let response = authenticate(client.get(url), protocol, key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| DiscoverError::Unreachable)?;
    match response.status().as_u16() {
        401 | 403 => return Err(DiscoverError::Unauthorized),
        404 | 405 | 501 => return Err(DiscoverError::NoList),
        200..=299 => {}
        _ => return Err(DiscoverError::Unreachable),
    }
    let value: Value = crate::oauth::bounded_json(response, 4 * 1024 * 1024)
        .await
        .map_err(|_| DiscoverError::NoList)?;
    let values = value["data"].as_array().ok_or(DiscoverError::NoList)?;
    let mut seen = std::collections::HashSet::new();
    let models: Vec<_> = values
        .iter()
        .filter_map(|model| {
            let id = model["id"].as_str()?.trim();
            if !crate::local::valid_model_id(id) || !seen.insert(id) {
                return None;
            }
            Some(LocalModel {
                id: id.to_owned(),
                context_window: model["max_input_tokens"].as_u64().filter(|n| *n > 0),
            })
        })
        .take(500)
        .collect();
    if models.is_empty() {
        return Err(DiscoverError::NoList);
    }
    Ok(Discovery {
        software: None,
        models,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_selected_address_and_protocol_own_the_key() {
        let mut route = LocalRoute {
            server: "synthetic".into(),
            base_url: "https://relay.example.test/custom/v1".into(),
            protocol: Some(CompatibleProtocol::Responses),
        };
        let key = credential_scope(&route).unwrap();
        assert_eq!(
            endpoint(&route.base_url, "models").unwrap(),
            "https://relay.example.test/custom/v1/models"
        );
        route.base_url.push('/');
        assert_eq!(credential_scope(&route).unwrap(), key);
        route.protocol = Some(CompatibleProtocol::AnthropicMessages);
        let second = credential_scope(&route).unwrap();
        assert_ne!(second, key);
        route.base_url = "https://elsewhere.example.test/v1".into();
        assert_ne!(credential_scope(&route).unwrap(), second);
    }

    #[test]
    fn remote_cleartext_and_urls_containing_credentials_are_refused() {
        for invalid in [
            "http://relay.example.test/v1",
            "https://user@relay.test/v1",
            "https://relay.test/v1?token=secret",
            "https://relay.test/#v1",
        ] {
            assert!(base_url(invalid).is_err(), "accepted {invalid}");
        }
        assert!(base_url("http://127.0.0.1:4000/v1").is_ok());
        assert!(base_url("http://[::1]:4000/v1").is_ok());
    }
}

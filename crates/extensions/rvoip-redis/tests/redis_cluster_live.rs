use std::{
    fs,
    process::Command,
    time::{Duration, SystemTime},
};

use rvoip_auth_core::{
    AuthAttemptAdmission, AuthAuditOutcome, AuthFailureReason, AuthRateLimitKey, AuthRateLimitKind,
    AuthRateLimiter, DigestNonceStatus, DigestReplayStore, TokenRevocationChecker,
    TokenRevocationContext, TokenRevocationStatus,
};
use rvoip_redis::{
    RedisAuthConfig, RedisAuthConnectionMode, RedisAuthError, RedisAuthProvider, RedisAuthTlsConfig,
};
use sha2::{Digest, Sha256};

// Keep this exhaustive downstream match as a source-compatibility sentinel
// for the released rvoip-redis 0.1.3 public error surface.
fn classify_public_error(error: RedisAuthError) -> &'static str {
    match error {
        RedisAuthError::Redis(_) => "redis",
        RedisAuthError::DurationTooLarge => "duration",
    }
}

#[test]
fn released_redis_auth_error_remains_exhaustively_matchable() {
    assert_eq!(
        classify_public_error(RedisAuthError::DurationTooLarge),
        "duration"
    );
}

fn cluster_seed_urls(variable: &str) -> Option<Vec<String>> {
    let raw = match std::env::var(variable) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return None,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("{variable} must contain valid Unicode")
        }
    };
    let entries = raw.split(',').map(str::trim).collect::<Vec<_>>();
    assert!(
        !entries.is_empty() && entries.iter().all(|url| !url.is_empty()),
        "{variable} was configured but contains an empty Redis Cluster seed"
    );
    Some(entries.into_iter().map(str::to_owned).collect())
}

fn unique_namespace(test_name: &str) -> String {
    format!(
        "rvoip:cluster-test:{test_name}:{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time must be after the Unix epoch")
            .as_nanos()
    )
}

fn provider_for_namespace(namespace: String, seed_urls: Vec<String>) -> RedisAuthProvider {
    let config = RedisAuthConfig::new(seed_urls[0].clone())
        .with_namespace(namespace)
        .with_nonce_stale_retention(Duration::from_secs(30))
        .with_nonce_count_ttl(Duration::from_secs(30))
        .with_token_revocation_ttl(Duration::from_secs(30))
        .with_rate_limit_window(Duration::from_secs(30))
        .with_max_failures_per_window(1)
        .with_max_initial_challenges_per_window(1);
    if seed_urls.iter().all(|url| url.starts_with("rediss://")) {
        RedisAuthProvider::from_cluster_config_with_tls(config, seed_urls, tls_config())
            .expect("configured TLS Redis Cluster provider must construct")
    } else {
        RedisAuthProvider::from_cluster_config(config, seed_urls)
            .expect("configured Redis Cluster provider must construct")
    }
}

fn cluster_provider_from_env(test_name: &str, variable: &str) -> Option<RedisAuthProvider> {
    let seed_urls = cluster_seed_urls(variable)?;
    Some(provider_for_namespace(
        unique_namespace(test_name),
        seed_urls,
    ))
}

fn cluster_provider(test_name: &str) -> Option<RedisAuthProvider> {
    cluster_provider_from_env(test_name, "RVOIP_REDIS_CLUSTER_URLS")
}

fn pem_from_env(variable: &str) -> Option<Vec<u8>> {
    let path = match std::env::var(variable) {
        Ok(path) => path,
        Err(std::env::VarError::NotPresent) => return None,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("{variable} must contain valid Unicode")
        }
    };
    assert!(!path.trim().is_empty(), "{variable} must not be empty");
    Some(fs::read(path).unwrap_or_else(|_| panic!("{variable} must name a readable PEM file")))
}

fn tls_config_with_root(root_certificate: Option<Vec<u8>>) -> RedisAuthTlsConfig {
    let mut tls = RedisAuthTlsConfig::new();
    if let Some(certificate) = root_certificate {
        tls = tls.with_root_certificate_pem(certificate);
    }
    match (
        pem_from_env("RVOIP_REDIS_TLS_CLIENT_CERT"),
        pem_from_env("RVOIP_REDIS_TLS_CLIENT_KEY"),
    ) {
        (Some(certificate), Some(private_key)) => {
            tls.with_client_identity_pem(certificate, private_key)
        }
        (None, None) => tls,
        _ => panic!("Redis TLS client certificate and key must be configured together"),
    }
}

fn tls_config() -> RedisAuthTlsConfig {
    tls_config_with_root(pem_from_env("RVOIP_REDIS_TLS_CA_CERT"))
}

fn assert_credentialed_rediss_urls(seed_urls: &[String], variable: &str) {
    assert!(
        seed_urls.iter().all(|url| {
            let Some(remainder) = url.strip_prefix("rediss://") else {
                return false;
            };
            let authority = remainder.split('/').next().unwrap_or_default();
            let Some((credentials, host)) = authority.rsplit_once('@') else {
                return false;
            };
            let password = credentials
                .split_once(':')
                .map(|(_, password)| password)
                .unwrap_or_default();
            !password.is_empty() && !host.is_empty()
        }),
        "{variable} must contain credentialed rediss:// seeds"
    );
}

#[derive(Clone, Debug)]
struct ClusterNode {
    id: String,
    port: u16,
    slots: Vec<(u16, u16)>,
}

impl ClusterNode {
    fn owns(&self, slot: u16) -> bool {
        self.slots
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&slot))
    }
}

struct DockerClusterFixture {
    container: String,
    password: String,
    ports: Vec<u16>,
    tls: bool,
}

impl DockerClusterFixture {
    fn from_env() -> Option<Self> {
        let container = match std::env::var("RVOIP_REDIS_CLUSTER_DOCKER_CONTAINER") {
            Ok(container) => container,
            Err(std::env::VarError::NotPresent) => return None,
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("RVOIP_REDIS_CLUSTER_DOCKER_CONTAINER must contain valid Unicode")
            }
        };
        assert!(!container.trim().is_empty(), "Docker container is empty");
        let password = std::env::var("RVOIP_REDIS_CLUSTER_PASSWORD")
            .expect("Docker-controlled cluster requires RVOIP_REDIS_CLUSTER_PASSWORD");
        assert!(!password.is_empty(), "Docker cluster password is empty");
        let raw_ports = std::env::var("RVOIP_REDIS_CLUSTER_PORTS")
            .expect("Docker-controlled cluster requires RVOIP_REDIS_CLUSTER_PORTS");
        let ports = raw_ports
            .split(',')
            .map(|port| {
                port.trim()
                    .parse::<u16>()
                    .expect("Docker cluster ports must be valid u16 values")
            })
            .collect::<Vec<_>>();
        assert!(ports.len() >= 3, "Docker cluster requires three primaries");
        let tls = match std::env::var("RVOIP_REDIS_CLUSTER_DOCKER_TLS") {
            Ok(value) if value == "true" => true,
            Ok(value) if value == "false" => false,
            Ok(_) => panic!("RVOIP_REDIS_CLUSTER_DOCKER_TLS must be true or false"),
            Err(std::env::VarError::NotPresent) => false,
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("RVOIP_REDIS_CLUSTER_DOCKER_TLS must contain valid Unicode")
            }
        };
        Some(Self {
            container,
            password,
            ports,
            tls,
        })
    }

    fn redis_cli(&self, port: u16, arguments: &[String]) -> String {
        let mut command = Command::new("docker");
        command
            .arg("exec")
            .arg("-e")
            .arg(format!("REDISCLI_AUTH={}", self.password))
            .arg(&self.container)
            .arg("redis-cli");
        if self.tls {
            command
                .arg("--tls")
                .arg("--cacert")
                .arg("/tls/ca.crt")
                .arg("--cert")
                .arg("/tls/client.crt")
                .arg("--key")
                .arg("/tls/client.key");
        }
        let output = command
            .arg("-p")
            .arg(port.to_string())
            .args(arguments)
            .output()
            .expect("docker exec redis-cli must start");
        assert!(
            output.status.success(),
            "redis-cli failed: {}",
            String::from_utf8_lossy(&output.stderr).replace(&self.password, "[REDACTED]")
        );
        String::from_utf8(output.stdout).expect("redis-cli output must be UTF-8")
    }

    fn master_nodes(&self) -> Vec<ClusterNode> {
        let output = self.redis_cli(self.ports[0], &["CLUSTER".into(), "NODES".into()]);
        let nodes = output
            .lines()
            .filter_map(|line| {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if fields.len() < 8 || !fields[2].split(',').any(|flag| flag == "master") {
                    return None;
                }
                let address = fields[1]
                    .split('@')
                    .next()
                    .expect("cluster node address must include a client address");
                let port = address
                    .rsplit(':')
                    .next()
                    .expect("cluster node address must include a port")
                    .parse::<u16>()
                    .expect("cluster node client port must be a u16");
                let slots = fields[8..]
                    .iter()
                    .filter(|slot| !slot.starts_with('['))
                    .map(|slot| {
                        let mut bounds = slot.split('-');
                        let start = bounds
                            .next()
                            .expect("slot range must have a start")
                            .parse::<u16>()
                            .expect("slot start must be a u16");
                        let end = bounds
                            .next()
                            .map(|end| end.parse::<u16>().expect("slot end must be a u16"))
                            .unwrap_or(start);
                        (start, end)
                    })
                    .collect();
                Some(ClusterNode {
                    id: fields[0].to_owned(),
                    port,
                    slots,
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(nodes.len(), 3, "fixture must expose three master nodes");
        nodes
    }

    fn count_keys_in_slot(&self, node: &ClusterNode, slot: u16) -> usize {
        self.redis_cli(
            node.port,
            &["CLUSTER".into(), "COUNTKEYSINSLOT".into(), slot.to_string()],
        )
        .trim()
        .parse()
        .expect("cluster slot key count must be numeric")
    }

    fn delete_keys_in_slot(&self, node: &ClusterNode, slot: u16) {
        let keys = self.redis_cli(
            node.port,
            &[
                "CLUSTER".into(),
                "GETKEYSINSLOT".into(),
                slot.to_string(),
                "1000".into(),
            ],
        );
        for key in keys.lines().filter(|key| !key.is_empty()) {
            self.redis_cli(node.port, &["DEL".into(), key.into()]);
        }
    }

    fn assign_slot(&self, slot: u16, source: &ClusterNode, target: &ClusterNode) {
        self.redis_cli(
            target.port,
            &[
                "CLUSTER".into(),
                "SETSLOT".into(),
                slot.to_string(),
                "IMPORTING".into(),
                source.id.clone(),
            ],
        );
        self.redis_cli(
            source.port,
            &[
                "CLUSTER".into(),
                "SETSLOT".into(),
                slot.to_string(),
                "MIGRATING".into(),
                target.id.clone(),
            ],
        );
        for port in &self.ports {
            self.redis_cli(
                *port,
                &[
                    "CLUSTER".into(),
                    "SETSLOT".into(),
                    slot.to_string(),
                    "NODE".into(),
                    target.id.clone(),
                ],
            );
        }
    }
}

struct SlotRestore<'a> {
    fixture: &'a DockerClusterFixture,
    slot: u16,
    original: ClusterNode,
    current: ClusterNode,
    armed: bool,
}

impl SlotRestore<'_> {
    fn restore(&mut self) {
        self.fixture.delete_keys_in_slot(&self.current, self.slot);
        self.fixture
            .assign_slot(self.slot, &self.current, &self.original);
        self.armed = false;
    }
}

impl Drop for SlotRestore<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.restore()));
        }
    }
}

fn digest_slot_key(namespace: &str) -> String {
    let digest = Sha256::digest(namespace.as_bytes());
    let tag = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{namespace}:{{{tag}}}:digest:nonce-expiry")
}

#[tokio::test]
async fn redis_cluster_routes_digest_lua_and_single_key_auth_state() {
    let Some(provider) = cluster_provider("auth-state") else {
        return;
    };
    assert_eq!(provider.connection_mode(), RedisAuthConnectionMode::Cluster);

    let nonce = provider
        .admit_nonce("cluster-nonce", SystemTime::now() + Duration::from_secs(20))
        .await
        .unwrap();
    assert_eq!(nonce, "cluster-nonce");
    assert_eq!(
        provider
            .nonce_status(&nonce, SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Active
    );
    assert!(provider
        .accept_nonce_count("alice", &nonce, 1)
        .await
        .unwrap());
    assert!(!provider
        .accept_nonce_count("alice", &nonce, 1)
        .await
        .unwrap());
    assert!(provider
        .accept_client_nonce_count("alice", &nonce, "secure-client", 1, SystemTime::now())
        .await
        .unwrap());

    // Exercise independently hash-tagged tenant namespaces so the test
    // traverses redirects and multiple primaries instead of proving Lua
    // execution for only one cluster slot.
    for shard in 0..12 {
        let shard_provider = cluster_provider(&format!("digest-slot-{shard}"))
            .expect("cluster seed configuration remains available");
        let shard_nonce = format!("cluster-nonce-{shard}");
        let admitted = shard_provider
            .admit_nonce(&shard_nonce, SystemTime::now() + Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(admitted, shard_nonce);
        assert!(shard_provider
            .accept_client_nonce_count("alice", &admitted, "secure-client", 1, SystemTime::now(),)
            .await
            .unwrap());
    }

    let token =
        TokenRevocationContext::new("cluster-token").with_issuer("https://issuer.example.test");
    assert_eq!(
        provider.check_token(&token).await.unwrap(),
        TokenRevocationStatus::Active
    );
    provider.revoke_token(&token).await.unwrap();
    assert_eq!(
        provider.check_token(&token).await.unwrap(),
        TokenRevocationStatus::Revoked
    );

    let rate_key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("alice")
        .with_realm("pbx.example.test")
        .with_peer("198.51.100.42");
    let AuthAttemptAdmission::Reserved(reservation) =
        provider.reserve_auth_attempt(&rate_key).await.unwrap()
    else {
        panic!("first cluster auth attempt must reserve capacity");
    };
    provider
        .complete_auth_attempt(
            &reservation,
            &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
        )
        .await
        .unwrap();
    assert!(matches!(
        provider.reserve_auth_attempt(&rate_key).await.unwrap(),
        AuthAttemptAdmission::Denied { .. }
    ));

    let first_challenge =
        AuthRateLimitKey::new(AuthRateLimitKind::SipChallenge).with_peer("198.51.100.43");
    let AuthAttemptAdmission::Reserved(challenge_reservation) = provider
        .reserve_auth_attempt(&first_challenge)
        .await
        .unwrap()
    else {
        panic!("first peer challenge must reserve capacity");
    };
    provider
        .complete_auth_attempt(
            &challenge_reservation,
            &AuthAuditOutcome::Failure(AuthFailureReason::MissingCredential),
        )
        .await
        .unwrap();
    assert!(matches!(
        provider
            .reserve_auth_attempt(
                &AuthRateLimitKey::new(AuthRateLimitKind::SipChallenge)
                    .with_realm("rotated.example.test")
                    .with_peer("198.51.100.43")
            )
            .await
            .unwrap(),
        AuthAttemptAdmission::Denied { .. }
    ));
    assert!(matches!(
        provider
            .reserve_auth_attempt(
                &AuthRateLimitKey::new(AuthRateLimitKind::SipChallenge).with_peer("198.51.100.44")
            )
            .await
            .unwrap(),
        AuthAttemptAdmission::Reserved(_)
    ));
}

#[tokio::test]
async fn docker_cluster_follows_moved_after_cached_topology_changes() {
    let Some(fixture) = DockerClusterFixture::from_env() else {
        return;
    };
    let seed_urls = cluster_seed_urls("RVOIP_REDIS_CLUSTER_URLS")
        .expect("Docker fixture requires Redis Cluster seeds");
    let nodes = fixture.master_nodes();
    let base_namespace = unique_namespace("moved-redirect");
    let (namespace, slot, source) = (0..16_384)
        .find_map(|candidate| {
            let namespace = format!("{base_namespace}:{candidate}");
            let slot = redis::cluster_routing::get_slot(digest_slot_key(&namespace).as_bytes());
            let source = nodes
                .iter()
                .find(|node| node.owns(slot))
                .expect("every Redis Cluster slot must have an owner")
                .clone();
            (fixture.count_keys_in_slot(&source, slot) == 0).then_some((namespace, slot, source))
        })
        .expect("fixture must contain an empty cluster slot");
    let target = nodes
        .iter()
        .find(|node| node.id != source.id)
        .expect("fixture must contain another primary")
        .clone();

    // One seed is deliberate: the provider must discover the cluster, cache
    // that original slot map, and then recover from the real MOVED response.
    let provider = provider_for_namespace(namespace, vec![seed_urls[0].clone()]);
    provider
        .admit_nonce(
            "before-slot-move",
            SystemTime::now() + Duration::from_secs(20),
        )
        .await
        .unwrap();
    assert!(fixture.count_keys_in_slot(&source, slot) > 0);
    fixture.delete_keys_in_slot(&source, slot);
    assert_eq!(fixture.count_keys_in_slot(&source, slot), 0);

    fixture.assign_slot(slot, &source, &target);
    let mut restore = SlotRestore {
        fixture: &fixture,
        slot,
        original: source,
        current: target.clone(),
        armed: true,
    };

    let nonce = provider
        .admit_nonce(
            "after-slot-move",
            SystemTime::now() + Duration::from_secs(20),
        )
        .await
        .expect("cached cluster connection must follow MOVED and refresh topology");
    assert_eq!(nonce, "after-slot-move");
    assert!(fixture.count_keys_in_slot(&target, slot) > 0);
    assert_eq!(
        provider
            .nonce_status(&nonce, SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Active
    );

    restore.restore();
}

#[tokio::test]
async fn authenticated_rediss_cluster_operates_when_configured() {
    let Some(seed_urls) = cluster_seed_urls("RVOIP_REDIS_CLUSTER_TLS_URLS") else {
        return;
    };
    assert_credentialed_rediss_urls(&seed_urls, "RVOIP_REDIS_CLUSTER_TLS_URLS");

    let provider = provider_for_namespace(unique_namespace("rediss"), seed_urls);
    assert_eq!(provider.connection_mode(), RedisAuthConnectionMode::Cluster);
    let nonce = provider
        .admit_nonce(
            "authenticated-rediss-cluster",
            SystemTime::now() + Duration::from_secs(20),
        )
        .await
        .expect("authenticated rediss cluster must accept Digest Lua operations");
    assert_eq!(nonce, "authenticated-rediss-cluster");
    assert_eq!(
        provider
            .nonce_status(&nonce, SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Active
    );
}

#[tokio::test]
async fn authenticated_rediss_single_node_operates_when_configured() {
    let url = match std::env::var("RVOIP_REDIS_SINGLE_TLS_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => return,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("RVOIP_REDIS_SINGLE_TLS_URL must contain valid Unicode")
        }
    };
    assert_credentialed_rediss_urls(std::slice::from_ref(&url), "RVOIP_REDIS_SINGLE_TLS_URL");
    let provider = RedisAuthProvider::from_config_with_tls(
        RedisAuthConfig::new(url).with_namespace(unique_namespace("single-rediss")),
        tls_config(),
    )
    .expect("configured TLS single-node Redis provider must construct");

    let nonce = provider
        .admit_nonce(
            "authenticated-single-rediss",
            SystemTime::now() + Duration::from_secs(20),
        )
        .await
        .expect("authenticated single-node rediss must accept Digest Lua operations");
    assert_eq!(nonce, "authenticated-single-rediss");
    assert_eq!(
        provider
            .nonce_status(&nonce, SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Active
    );
}

#[tokio::test]
async fn untrusted_ca_is_rejected_for_single_node_and_cluster() {
    let Some(untrusted_root) = pem_from_env("RVOIP_REDIS_TLS_UNTRUSTED_CA_CERT") else {
        return;
    };
    let single_url = std::env::var("RVOIP_REDIS_SINGLE_TLS_URL")
        .expect("untrusted-CA fixture requires RVOIP_REDIS_SINGLE_TLS_URL");
    let seed_urls = cluster_seed_urls("RVOIP_REDIS_CLUSTER_TLS_URLS")
        .expect("untrusted-CA fixture requires RVOIP_REDIS_CLUSTER_TLS_URLS");

    let single = RedisAuthProvider::from_config_with_tls(
        RedisAuthConfig::new(single_url).with_namespace(unique_namespace("untrusted-single")),
        tls_config_with_root(Some(untrusted_root.clone())),
    )
    .expect("untrusted root is syntactically valid");
    assert!(single
        .admit_nonce(
            "must-not-reach-untrusted-single",
            SystemTime::now() + Duration::from_secs(20),
        )
        .await
        .is_err());

    let cluster_config = RedisAuthConfig::new(seed_urls[0].clone())
        .with_namespace(unique_namespace("untrusted-cluster"));
    let cluster = RedisAuthProvider::from_cluster_config_with_tls(
        cluster_config,
        seed_urls,
        tls_config_with_root(Some(untrusted_root)),
    )
    .expect("untrusted root is syntactically valid");
    assert!(cluster
        .admit_nonce(
            "must-not-reach-untrusted-cluster",
            SystemTime::now() + Duration::from_secs(20),
        )
        .await
        .is_err());
}

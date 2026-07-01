//! Ingress reconciliation for webhook-based platforms (Telegram, LINE, ...).
//!
//! Implements Option 1 of `docs/refarch/running-telegram-line-on-aws.md`:
//! API Gateway HTTP API → VPC Link → Cloud Map → ECS Fargate task on `:port`.
//!
//! All operations are idempotent — resources are looked up by name and reused,
//! so repeated `oabctl apply` runs converge instead of duplicating. The shared
//! VPC Link and Cloud Map namespace are scoped per-VPC (their names include the
//! VPC ID) since a VPC Link's ENIs and a namespace's private DNS are only valid
//! within the VPC they were created in — two VPCs never share either resource.
//!
//! The reconciliation is split in two because ECS service discovery
//! (`--service-registries`) can only be set at service *creation* time:
//!   1. [`ensure_cloud_map`] — namespace + service. Must run BEFORE the ECS
//!      service is created so its registry ARN can be attached.
//!   2. [`ensure_gateway`] — VPC Link + HTTP API + integration + routes + stage
//!      + security-group inbound rule. Runs AFTER the task is wired up.

use crate::manifest::{Ingress, OABServiceManifest};
use anyhow::{Context, Result};
use aws_sdk_apigatewayv2::types::{ConnectionType, IntegrationType, ProtocolType};
use aws_sdk_servicediscovery::types::{DnsConfig, DnsRecord, RecordType};

const STAGE_NAME: &str = "prod";

/// VPC Link name, scoped per-VPC. A VPC Link's ENIs live in one VPC and cannot
/// route to another, so each VPC gets its own link — sharing a name across VPCs
/// would silently misroute traffic through the wrong VPC's link.
fn vpc_link_name(vpc_id: &str) -> String {
    format!("oab-vpc-link-{vpc_id}")
}

/// Cloud Map private DNS namespace name, scoped per-VPC. A namespace's private
/// DNS only resolves within the VPC it's associated with, so two VPCs both
/// using the configured `cloudMapNamespace` (default `oab`) must not resolve to
/// the same lookup — scope the actual namespace by VPC ID.
fn vpc_scoped_namespace(configured_namespace: &str, vpc_id: &str) -> String {
    format!("{configured_namespace}-{vpc_id}")
}

/// Per-bot HTTP API name. Each ingress bot gets its own API so webhook paths
/// (e.g. `/webhook/telegram`) can never collide between bots on a shared API.
fn api_name(namespace: &str, name: &str) -> String {
    format!("oab-webhook-{namespace}-{name}")
}

/// Result of Cloud Map reconciliation, consumed when creating the ECS service.
pub struct CloudMapResult {
    /// Cloud Map service ARN — used both as the ECS service registry ARN and as
    /// the API Gateway integration URI.
    pub registry_arn: String,
}

/// Step 1: ensure the Cloud Map private DNS namespace and service exist.
///
/// Returns the registry ARN to attach to the ECS service and the DNS name the
/// API Gateway integration will target.
pub async fn ensure_cloud_map(
    config: &aws_config::SdkConfig,
    m: &OABServiceManifest,
    ingress: &Ingress,
) -> Result<CloudMapResult> {
    let sd = aws_sdk_servicediscovery::Client::new(config);
    let ec2 = aws_sdk_ec2::Client::new(config);

    let vpc_id = resolve_vpc_id(&ec2, m).await?;

    // ── Namespace (shared per-VPC; scoped by VPC so two VPCs using the same
    //    configured cloudMapNamespace name never collide — a namespace's DNS
    //    is only resolvable within the VPC it's associated with) ────────────
    let namespace_name = vpc_scoped_namespace(&ingress.cloud_map_namespace, &vpc_id);
    let namespace_id = ensure_namespace(&sd, &namespace_name, &vpc_id).await?;

    // ── Service (one per bot) ──────────────────────────────────────────────
    let service_name = m.cloud_map_service_name();
    let (registry_arn, existed) = ensure_service(&sd, &namespace_id, &service_name).await?;

    let dns_name = format!("{service_name}.{namespace_name}");
    if existed {
        eprintln!("  ✓ Cloud Map service exists: {dns_name}");
    } else {
        eprintln!("  ✓ Created Cloud Map service: {dns_name}");
    }

    Ok(CloudMapResult { registry_arn })
}

/// Step 2: ensure VPC Link, HTTP API, integration, routes, stage, and the
/// security-group inbound rule. Returns the public webhook URLs (one per path).
pub async fn ensure_gateway(
    config: &aws_config::SdkConfig,
    namespace: &str,
    name: &str,
    ingress: &Ingress,
    subnets: &[String],
    security_groups: &[String],
    cloud_map_service_arn: &str,
) -> Result<Vec<String>> {
    let api = aws_sdk_apigatewayv2::Client::new(config);
    let ec2 = aws_sdk_ec2::Client::new(config);
    let api_name = api_name(namespace, name);

    // ── Security group inbound rule (self-referencing on the container port) ─
    ensure_sg_ingress(&ec2, security_groups, ingress.container_port).await?;

    // ── VPC Link (shared per-VPC, waits for AVAILABLE) ──────────────────────
    let subnet = subnets.first().context("ingress requires at least one subnet")?;
    let vpc_id = resolve_vpc_id_from_subnet(&ec2, subnet).await?;
    let vpc_link_id = ensure_vpc_link(&api, &vpc_id, subnets, security_groups).await?;

    // ── HTTP API (one per bot — avoids cross-bot path collisions) ──────────
    let (api_id, api_endpoint) = ensure_api(&api, &api_name).await?;

    // ── Integration: VPC Link → Cloud Map service (URI is the service ARN;
    //    the port is resolved from the service's SRV record) ───────────────
    let integration_id =
        ensure_integration(&api, &api_id, &vpc_link_id, cloud_map_service_arn).await?;

    // ── One route per webhook path, all → the same integration ─────────────
    for path in &ingress.paths {
        ensure_route(&api, &api_id, path, &integration_id).await?;
    }

    // ── Prune routes for paths no longer in the manifest (rename/removal) ───
    prune_stale_routes(&api, &api_id, &ingress.paths).await?;

    // ── Stage (auto-deploy) ────────────────────────────────────────────────
    ensure_stage(&api, &api_id).await?;

    Ok(webhook_urls(&api_endpoint, &ingress.paths))
}

/// API Gateway route key for a webhook path (POST only).
fn route_key(path: &str) -> String {
    format!("POST {path}")
}

/// Build the public webhook URL(s) from the API endpoint and paths.
/// Each URL is `<endpoint>/<stage><path>`.
fn webhook_urls(api_endpoint: &str, paths: &[String]) -> Vec<String> {
    let base = api_endpoint.trim_end_matches('/');
    paths
        .iter()
        .map(|p| format!("{base}/{STAGE_NAME}{p}"))
        .collect()
}

/// Best-effort teardown of the *per-bot* ingress resources for `namespace/name`:
/// the bot's own HTTP API (`oab-webhook-<ns>-<name>`, which cascades its routes,
/// integration and stage) and its Cloud Map service.
///
/// The shared resources (the `oab-vpc-link` VPC Link and the security-group
/// inbound rule) are intentionally left in place since other bots may still use
/// them. Safe to call for bots that never had ingress — it simply finds nothing
/// and returns. Errors are logged, not propagated, so teardown never blocks
/// service deletion.
pub async fn teardown(config: &aws_config::SdkConfig, namespace: &str, name: &str) -> Result<()> {
    let service_name = format!("oab-{namespace}-{name}");
    let api = aws_sdk_apigatewayv2::Client::new(config);

    // ── API Gateway: delete the whole per-bot API (cascades routes/integration/stage) ─
    if let Some((api_id, _)) = find_api(&api, &api_name(namespace, name)).await? {
        match api.delete_api().api_id(&api_id).send().await {
            Ok(_) => eprintln!("  ✓ Deleted HTTP API: {}", api_name(namespace, name)),
            Err(e) => eprintln!("  ⚠ Failed to delete HTTP API {api_id}: {e}"),
        }
    }

    // ── Cloud Map: delete the per-bot service (needs no live instances) ──────
    let sd = aws_sdk_servicediscovery::Client::new(config);
    let mut service_id: Option<String> = None;
    let mut pages = sd.list_services().into_paginator().send();
    'svc: while let Some(page) = pages.next().await {
        let page = page.context("failed to list Cloud Map services")?;
        for s in page.services() {
            if s.name() == Some(service_name.as_str()) {
                service_id = s.id().map(|x| x.to_string());
                break 'svc;
            }
        }
    }
    if let Some(service_id) = service_id {
        match sd.delete_service().id(&service_id).send().await {
            Ok(_) => eprintln!("  ✓ Deleted Cloud Map service: {service_name}"),
            Err(e) => eprintln!(
                "  ⚠ Cloud Map service '{service_name}' not deleted — it may still have\n    registered instances; retry once the ECS tasks have fully drained ({e})"
            ),
        }
    }

    Ok(())
}

// ─── VPC resolution ─────────────────────────────────────────────────────────

async fn resolve_vpc_id(ec2: &aws_sdk_ec2::Client, m: &OABServiceManifest) -> Result<String> {
    let subnet = match &m.spec.runtime {
        crate::manifest::Runtime::Ecs(rt) => rt
            .networking
            .subnets
            .first()
            .context("ingress requires at least one subnet")?,
        _ => anyhow::bail!("ingress is only supported for ECS runtime"),
    };
    resolve_vpc_id_from_subnet(ec2, subnet).await
}

/// Resolve the VPC ID that a given subnet belongs to.
async fn resolve_vpc_id_from_subnet(ec2: &aws_sdk_ec2::Client, subnet: &str) -> Result<String> {
    let resp = ec2
        .describe_subnets()
        .subnet_ids(subnet)
        .send()
        .await
        .with_context(|| format!("failed to describe subnet {subnet}"))?;
    let vpc_id = resp
        .subnets()
        .first()
        .and_then(|s| s.vpc_id())
        .with_context(|| format!("subnet {subnet} has no VPC"))?
        .to_string();
    Ok(vpc_id)
}

// ─── Cloud Map ────────────────────────────────────────────────────────────────

async fn ensure_namespace(
    sd: &aws_sdk_servicediscovery::Client,
    name: &str,
    vpc_id: &str,
) -> Result<String> {
    // Reuse an existing private DNS namespace with this name if present.
    let mut pages = sd.list_namespaces().into_paginator().send();
    while let Some(page) = pages.next().await {
        let page = page.context("failed to list Cloud Map namespaces")?;
        for ns in page.namespaces() {
            if ns.name() == Some(name) {
                let id = ns.id().context("namespace missing id")?.to_string();
                eprintln!("  ✓ Cloud Map namespace exists: {name}");
                return Ok(id);
            }
        }
    }

    // Create — this is an async Cloud Map operation; poll until it completes.
    eprintln!("  ⊕ Creating Cloud Map namespace: {name} (VPC {vpc_id})");
    let out = sd
        .create_private_dns_namespace()
        .name(name)
        .vpc(vpc_id)
        .send()
        .await
        .context("failed to create Cloud Map namespace")?;
    let op_id = out
        .operation_id()
        .context("no operation id for namespace creation")?;
    let namespace_id = wait_for_operation_target(sd, op_id, "NAMESPACE").await?;
    Ok(namespace_id)
}

async fn ensure_service(
    sd: &aws_sdk_servicediscovery::Client,
    namespace_id: &str,
    service_name: &str,
) -> Result<(String, bool)> {
    // Look for an existing service in this namespace with the given name.
    let filter = aws_sdk_servicediscovery::types::ServiceFilter::builder()
        .name(aws_sdk_servicediscovery::types::ServiceFilterName::NamespaceId)
        .values(namespace_id)
        .build()
        .context("failed to build service filter")?;
    let mut pages = sd.list_services().filters(filter).into_paginator().send();
    while let Some(page) = pages.next().await {
        let page = page.context("failed to list Cloud Map services")?;
        for svc in page.services() {
            if svc.name() == Some(service_name) {
                let arn = svc.arn().context("service missing arn")?.to_string();
                return Ok((arn, true));
            }
        }
    }

    // Create with an SRV record. For an HTTP API private integration whose URI
    // is the Cloud Map service ARN, API Gateway learns the target port from the
    // SRV record — a plain A record carries no port and does NOT work. ECS
    // registers the task's IP + container port into this SRV record.
    let dns_record = DnsRecord::builder()
        .r#type(RecordType::Srv)
        .ttl(60)
        .build()
        .context("failed to build DNS record")?;
    let dns_config = DnsConfig::builder()
        .dns_records(dns_record)
        .build()
        .context("failed to build DNS config")?;
    let out = sd
        .create_service()
        .name(service_name)
        .namespace_id(namespace_id)
        .dns_config(dns_config)
        .send()
        .await
        .context("failed to create Cloud Map service")?;
    let arn = out
        .service()
        .and_then(|s| s.arn())
        .context("created service has no ARN")?
        .to_string();
    Ok((arn, false))
}

async fn wait_for_operation_target(
    sd: &aws_sdk_servicediscovery::Client,
    op_id: &str,
    target_key: &str,
) -> Result<String> {
    use aws_sdk_servicediscovery::types::OperationStatus;
    for _ in 0..60 {
        let resp = sd
            .get_operation()
            .operation_id(op_id)
            .send()
            .await
            .context("failed to poll Cloud Map operation")?;
        let op = resp.operation().context("no operation in response")?;
        match op.status() {
            Some(OperationStatus::Success) => {
                let target = op
                    .targets()
                    .and_then(|t| {
                        t.iter()
                            .find(|(k, _)| k.as_str() == target_key)
                            .map(|(_, v)| v.clone())
                    })
                    .context("operation succeeded but target id missing")?;
                return Ok(target);
            }
            Some(OperationStatus::Fail) => {
                anyhow::bail!(
                    "Cloud Map operation failed: {}",
                    op.error_message().unwrap_or("unknown error")
                );
            }
            _ => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
        }
    }
    anyhow::bail!("timed out waiting for Cloud Map operation {op_id}")
}

// ─── Security group ───────────────────────────────────────────────────────────

async fn ensure_sg_ingress(
    ec2: &aws_sdk_ec2::Client,
    security_groups: &[String],
    port: u16,
) -> Result<()> {
    use aws_sdk_ec2::error::ProvideErrorMetadata;
    use aws_sdk_ec2::types::{IpPermission, UserIdGroupPair};
    for sg in security_groups {
        // Self-referencing rule: VPC Link ENIs live in this SG, so allowing the
        // SG to reach itself on the container port covers VPC Link → task.
        let pair = UserIdGroupPair::builder().group_id(sg).build();
        let perm = IpPermission::builder()
            .ip_protocol("tcp")
            .from_port(port as i32)
            .to_port(port as i32)
            .user_id_group_pairs(pair)
            .build();
        match ec2
            .authorize_security_group_ingress()
            .group_id(sg)
            .ip_permissions(perm)
            .send()
            .await
        {
            Ok(_) => eprintln!("  ✓ SG {sg}: allowed self :{port} (VPC Link → task)"),
            // EC2 returns InvalidPermission.Duplicate when the rule already exists.
            // Match the typed error code, not the Debug-rendered message text.
            Err(e) if e.code() == Some("InvalidPermission.Duplicate") => {
                eprintln!("  ✓ SG {sg}: inbound :{port} rule already present");
            }
            Err(e) => {
                return Err(anyhow::anyhow!(e))
                    .with_context(|| format!("failed to authorize ingress on {sg}"));
            }
        }
    }
    Ok(())
}

// ─── VPC Link ─────────────────────────────────────────────────────────────────

async fn ensure_vpc_link(
    api: &aws_sdk_apigatewayv2::Client,
    vpc_id: &str,
    subnets: &[String],
    security_groups: &[String],
) -> Result<String> {
    use aws_sdk_apigatewayv2::types::VpcLinkStatus;
    let link_name = vpc_link_name(vpc_id);

    // Reuse an existing, non-failed VPC Link with our per-VPC name.
    let mut found: Option<(String, Option<VpcLinkStatus>)> = None;
    let mut next: Option<String> = None;
    'outer: loop {
        let mut req = api.get_vpc_links();
        if let Some(t) = &next {
            req = req.next_token(t);
        }
        let resp = req.send().await.context("failed to list VPC Links")?;
        for link in resp.items() {
            if link.name() == Some(link_name.as_str()) {
                if matches!(
                    link.vpc_link_status(),
                    Some(VpcLinkStatus::Failed) | Some(VpcLinkStatus::Deleting)
                ) {
                    continue;
                }
                found = Some((
                    link.vpc_link_id().unwrap_or_default().to_string(),
                    link.vpc_link_status().cloned(),
                ));
                break 'outer;
            }
        }
        match resp.next_token() {
            Some(t) => next = Some(t.to_string()),
            None => break,
        }
    }

    let link_id = if let Some((id, _status)) = found {
        eprintln!("  ✓ VPC Link exists: {link_name} ({id})");
        // A VPC Link's subnets/SGs are fixed at creation and cannot be updated.
        // All ingress-enabled bots in this VPC share this one link, so they must
        // use the same subnet/SG set as whichever bot created it first —
        // otherwise the link's ENIs won't cover this task's subnets and
        // integrations may 503.
        eprintln!(
            "    ↳ reusing this VPC's shared link subnets/SGs (fixed at creation);\n      ensure all ingress bots in vpc {vpc_id} use the same subnets/securityGroups"
        );
        id
    } else {
        eprintln!("  ⊕ Creating VPC Link: {link_name}");
        let out = api
            .create_vpc_link()
            .name(&link_name)
            .set_subnet_ids(Some(subnets.to_vec()))
            .set_security_group_ids(Some(security_groups.to_vec()))
            .send()
            .await
            .context("failed to create VPC Link")?;
        out.vpc_link_id().context("no VPC Link id")?.to_string()
    };

    // Wait until AVAILABLE — routes won't serve traffic while PENDING.
    for _ in 0..60 {
        let resp = api
            .get_vpc_link()
            .vpc_link_id(&link_id)
            .send()
            .await
            .context("failed to poll VPC Link")?;
        match resp.vpc_link_status() {
            Some(VpcLinkStatus::Available) => return Ok(link_id),
            Some(VpcLinkStatus::Failed) => anyhow::bail!(
                "VPC Link {link_id} entered FAILED state: {}",
                resp.vpc_link_status_message().unwrap_or("unknown")
            ),
            _ => {
                eprintln!("    … waiting for VPC Link to become AVAILABLE (can take a few min)");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
    }
    anyhow::bail!("timed out waiting for VPC Link {link_id} to become AVAILABLE")
}

// ─── HTTP API ───────────────────────────────────────────────────────────────

async fn ensure_api(
    api: &aws_sdk_apigatewayv2::Client,
    api_name: &str,
) -> Result<(String, String)> {
    if let Some((id, endpoint)) = find_api(api, api_name).await? {
        eprintln!("  ✓ HTTP API exists: {api_name} ({id})");
        return Ok((id, endpoint));
    }
    eprintln!("  ⊕ Creating HTTP API: {api_name}");
    let out = api
        .create_api()
        .name(api_name)
        .protocol_type(ProtocolType::Http)
        .send()
        .await
        .context("failed to create HTTP API")?;
    let id = out.api_id().context("no api id")?.to_string();
    let endpoint = out.api_endpoint().unwrap_or_default().to_string();
    Ok((id, endpoint))
}

/// Find an HTTP API by name, returning `(api_id, api_endpoint)`.
async fn find_api(
    api: &aws_sdk_apigatewayv2::Client,
    api_name: &str,
) -> Result<Option<(String, String)>> {
    // apigatewayv2 has no smithy paginator for GetApis; page manually.
    let mut next: Option<String> = None;
    loop {
        let mut req = api.get_apis();
        if let Some(t) = &next {
            req = req.next_token(t);
        }
        let resp = req.send().await.context("failed to list APIs")?;
        for a in resp.items() {
            if a.name() == Some(api_name) {
                let id = a.api_id().context("api missing id")?.to_string();
                let endpoint = a.api_endpoint().unwrap_or_default().to_string();
                return Ok(Some((id, endpoint)));
            }
        }
        match resp.next_token() {
            Some(t) => next = Some(t.to_string()),
            None => return Ok(None),
        }
    }
}

async fn ensure_integration(
    api: &aws_sdk_apigatewayv2::Client,
    api_id: &str,
    vpc_link_id: &str,
    integration_uri: &str,
) -> Result<String> {
    let mut next: Option<String> = None;
    loop {
        let mut req = api.get_integrations().api_id(api_id);
        if let Some(t) = &next {
            req = req.next_token(t);
        }
        let resp = req.send().await.context("failed to list integrations")?;
        for i in resp.items() {
            if i.integration_uri() == Some(integration_uri) && i.connection_id() == Some(vpc_link_id)
            {
                let id = i
                    .integration_id()
                    .context("integration missing id")?
                    .to_string();
                eprintln!("  ✓ Integration exists → {integration_uri}");
                return Ok(id);
            }
        }
        match resp.next_token() {
            Some(t) => next = Some(t.to_string()),
            None => break,
        }
    }
    eprintln!("  ⊕ Creating integration → {integration_uri}");
    let out = api
        .create_integration()
        .api_id(api_id)
        .integration_type(IntegrationType::HttpProxy)
        .integration_method("ANY")
        .integration_uri(integration_uri)
        .connection_type(ConnectionType::VpcLink)
        .connection_id(vpc_link_id)
        .payload_format_version("1.0")
        .send()
        .await
        .context("failed to create integration")?;
    Ok(out
        .integration_id()
        .context("no integration id")?
        .to_string())
}

async fn ensure_route(
    api: &aws_sdk_apigatewayv2::Client,
    api_id: &str,
    path: &str,
    integration_id: &str,
) -> Result<()> {
    let route_key = route_key(path);
    let target = format!("integrations/{integration_id}");
    let mut next: Option<String> = None;
    loop {
        let mut req = api.get_routes().api_id(api_id);
        if let Some(t) = &next {
            req = req.next_token(t);
        }
        let resp = req.send().await.context("failed to list routes")?;
        for r in resp.items() {
            if r.route_key() == Some(route_key.as_str()) {
                eprintln!("  ✓ Route exists: {route_key}");
                return Ok(());
            }
        }
        match resp.next_token() {
            Some(t) => next = Some(t.to_string()),
            None => break,
        }
    }
    api.create_route()
        .api_id(api_id)
        .route_key(&route_key)
        .target(&target)
        .send()
        .await
        .with_context(|| format!("failed to create route {route_key}"))?;
    eprintln!("  ⊕ Created route: {route_key}");
    Ok(())
}

/// Delete any route on the bot's API whose path isn't in `current_paths`.
///
/// `ensure_route` only ever adds routes; without this, renaming or removing a
/// webhook path in the manifest leaves a dead route on the API permanently.
async fn prune_stale_routes(
    api: &aws_sdk_apigatewayv2::Client,
    api_id: &str,
    current_paths: &[String],
) -> Result<()> {
    let current_keys: std::collections::HashSet<String> =
        current_paths.iter().map(|p| route_key(p)).collect();

    let mut stale: Vec<(String, String)> = Vec::new(); // (route_id, route_key)
    let mut next: Option<String> = None;
    loop {
        let mut req = api.get_routes().api_id(api_id);
        if let Some(t) = &next {
            req = req.next_token(t);
        }
        let resp = req.send().await.context("failed to list routes")?;
        for r in resp.items() {
            if let Some(key) = r.route_key() {
                if !current_keys.contains(key) {
                    if let Some(id) = r.route_id() {
                        stale.push((id.to_string(), key.to_string()));
                    }
                }
            }
        }
        match resp.next_token() {
            Some(t) => next = Some(t.to_string()),
            None => break,
        }
    }

    for (route_id, key) in stale {
        match api.delete_route().api_id(api_id).route_id(&route_id).send().await {
            Ok(_) => eprintln!("  ⊖ Removed stale route (no longer in manifest): {key}"),
            Err(e) => eprintln!("  ⚠ Failed to remove stale route {key}: {e}"),
        }
    }
    Ok(())
}

async fn ensure_stage(api: &aws_sdk_apigatewayv2::Client, api_id: &str) -> Result<()> {
    let mut next: Option<String> = None;
    loop {
        let mut req = api.get_stages().api_id(api_id);
        if let Some(t) = &next {
            req = req.next_token(t);
        }
        let resp = req.send().await.context("failed to list stages")?;
        for s in resp.items() {
            if s.stage_name() == Some(STAGE_NAME) {
                eprintln!("  ✓ Stage exists: {STAGE_NAME}");
                return Ok(());
            }
        }
        match resp.next_token() {
            Some(t) => next = Some(t.to_string()),
            None => break,
        }
    }
    api.create_stage()
        .api_id(api_id)
        .stage_name(STAGE_NAME)
        .auto_deploy(true)
        .send()
        .await
        .context("failed to create stage")?;
    eprintln!("  ⊕ Created stage: {STAGE_NAME} (auto-deploy)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_key_is_post_prefixed() {
        assert_eq!(route_key("/webhook/telegram"), "POST /webhook/telegram");
        assert_eq!(route_key("/webhook/line"), "POST /webhook/line");
    }

    #[test]
    fn api_name_is_per_bot() {
        assert_eq!(api_name("prod", "mybot"), "oab-webhook-prod-mybot");
        assert_ne!(api_name("prod", "a"), api_name("prod", "b"));
    }

    #[test]
    fn vpc_link_name_is_per_vpc() {
        assert_eq!(vpc_link_name("vpc-abc123"), "oab-vpc-link-vpc-abc123");
        assert_ne!(vpc_link_name("vpc-aaa"), vpc_link_name("vpc-bbb"));
    }

    #[test]
    fn vpc_scoped_namespace_differs_per_vpc() {
        assert_eq!(vpc_scoped_namespace("oab", "vpc-aaa"), "oab-vpc-aaa");
        assert_ne!(
            vpc_scoped_namespace("oab", "vpc-aaa"),
            vpc_scoped_namespace("oab", "vpc-bbb")
        );
        // Same VPC, different configured namespace names still differ.
        assert_ne!(
            vpc_scoped_namespace("oab", "vpc-aaa"),
            vpc_scoped_namespace("custom", "vpc-aaa")
        );
    }

    #[test]
    fn webhook_urls_join_endpoint_stage_and_path() {
        let paths = vec![
            "/webhook/telegram".to_string(),
            "/webhook/line".to_string(),
        ];
        let urls = webhook_urls("https://abc123.execute-api.us-east-1.amazonaws.com", &paths);
        assert_eq!(
            urls,
            vec![
                "https://abc123.execute-api.us-east-1.amazonaws.com/prod/webhook/telegram",
                "https://abc123.execute-api.us-east-1.amazonaws.com/prod/webhook/line",
            ]
        );
    }

    #[test]
    fn webhook_urls_trim_trailing_slash_on_endpoint() {
        let paths = vec!["/webhook/telegram".to_string()];
        let urls = webhook_urls("https://abc123.example.com/", &paths);
        assert_eq!(urls, vec!["https://abc123.example.com/prod/webhook/telegram"]);
    }
}

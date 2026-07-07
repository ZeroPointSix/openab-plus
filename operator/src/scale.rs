use anyhow::{Context, Result};

/// Resolve an alias (or bare name) to (cluster, service_name).
/// Supports:
///   - ecsctl alias format: "cluster/service[/container[/task_id]]"
///   - bare agent name: resolved as "oab-{namespace}-{name}" in the config cluster
async fn resolve_service(
    aws_config: &aws_config::SdkConfig,
    alias_or_name: &str,
) -> Result<(String, String)> {
    let ecsctl_cfg = ecsctl::config::Config::load().unwrap_or_default();

    // Check if it's an ecsctl alias
    if let Some(target) = ecsctl_cfg.aliases.get(alias_or_name) {
        let parts: Vec<&str> = target.splitn(4, '/').collect();
        if parts.len() >= 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
    }

    // Not an alias — treat as bare agent name
    eprintln!(
        "  note: '{}' not found in ecsctl aliases, resolving as bare agent name",
        alias_or_name
    );
    let oab_cfg =
        crate::config::OabConfig::load().context("failed to load ~/.oabctl/config.toml")?;
    let cluster = oab_cfg.defaults.cluster;
    let namespace = oab_cfg.defaults.namespace;
    let service_name = format!("oab-{}-{}", namespace, alias_or_name);

    // Verify the service exists
    let ecs = aws_sdk_ecs::Client::new(aws_config);
    let resp = ecs
        .describe_services()
        .cluster(&cluster)
        .services(&service_name)
        .send()
        .await
        .context("failed to describe ECS service")?;

    let svc = resp.services().first().context(format!(
        "service '{}' not found in cluster '{}'. Use 'oabctl get services' to list available services.",
        service_name, cluster
    ))?;

    let status = svc.status().unwrap_or("UNKNOWN");
    if status == "INACTIVE" || status == "DRAINING" {
        anyhow::bail!(
            "service '{}' is {} — cannot scale",
            service_name,
            status
        );
    }

    Ok((cluster, service_name))
}

/// Immediate scale: update desired count directly.
pub async fn run(aws_config: &aws_config::SdkConfig, alias: &str, size: i32) -> Result<()> {
    let (cluster, service_name) = resolve_service(aws_config, alias).await?;
    let ecs = aws_sdk_ecs::Client::new(aws_config);

    ecs.update_service()
        .cluster(&cluster)
        .service(&service_name)
        .desired_count(size)
        .send()
        .await
        .context("failed to update ECS service desired count")?;

    println!("✓ Scaled {alias} ({service_name}) to {size} in cluster {cluster}");
    Ok(())
}

/// Build the scheduler Target for ECS UpdateService.
fn build_schedule_target(
    role_arn: &str,
    target_input: &str,
) -> Result<aws_sdk_scheduler::types::Target> {
    aws_sdk_scheduler::types::Target::builder()
        .arn("arn:aws:scheduler:::aws-sdk:ecs:updateService")
        .role_arn(role_arn)
        .input(target_input)
        .build()
        .context("failed to build scheduler target")
}

/// Build the FlexibleTimeWindow (OFF mode).
fn build_flexible_time_window() -> Result<aws_sdk_scheduler::types::FlexibleTimeWindow> {
    aws_sdk_scheduler::types::FlexibleTimeWindow::builder()
        .mode(aws_sdk_scheduler::types::FlexibleTimeWindowMode::Off)
        .build()
        .context("failed to build flexible time window")
}

/// Check if a schedule exists. Returns true only for confirmed existence;
/// returns false for ResourceNotFoundException; propagates other errors.
async fn schedule_exists(
    scheduler: &aws_sdk_scheduler::Client,
    name: &str,
    group_name: &str,
) -> Result<bool> {
    match scheduler
        .get_schedule()
        .name(name)
        .group_name(group_name)
        .send()
        .await
    {
        Ok(_) => Ok(true),
        Err(e) => {
            let service_err = e.as_service_error();
            if service_err
                .map(|se| se.is_resource_not_found_exception())
                .unwrap_or(false)
            {
                Ok(false)
            } else {
                Err(e).context(format!("failed to check if schedule '{}' exists", name))
            }
        }
    }
}

/// Scheduled scale: create an EventBridge Scheduler schedule that calls
/// ECS UpdateService at the given schedule expression.
pub async fn run_with_schedule(
    aws_config: &aws_config::SdkConfig,
    alias: &str,
    size: i32,
    schedule_expression: &str,
    timezone: Option<&str>,
) -> Result<()> {
    let (cluster, service_name) = resolve_service(aws_config, alias).await?;
    let scheduler = aws_sdk_scheduler::Client::new(aws_config);
    let sts = aws_sdk_sts::Client::new(aws_config);
    let iam = aws_sdk_iam::Client::new(aws_config);

    // Get account ID for ARN construction
    let identity = sts
        .get_caller_identity()
        .send()
        .await
        .context("failed to get caller identity")?;
    let account_id = identity.account().context("no account ID")?;
    let region = aws_config
        .region()
        .map(|r| r.as_ref().to_string())
        .unwrap_or_else(|| "us-east-1".to_string());

    // Ensure schedule group exists
    let group_name = "oab-schedules";
    ensure_schedule_group(&scheduler, group_name).await?;

    // Ensure scheduler IAM role exists
    let role_arn =
        ensure_scheduler_role(&iam, account_id, &region, &cluster).await?;

    // Build schedule name: oab-scale-{alias}-to-{size}
    // Replace non-alphanumeric chars for valid schedule name
    let safe_alias = alias.replace(|c: char| !c.is_alphanumeric() && c != '-', "-");
    let schedule_name = format!("oab-scale-{}-to-{}", safe_alias, size);

    // Build the ECS UpdateService input for the universal target
    let target_input = serde_json::json!({
        "Cluster": cluster,
        "Service": service_name,
        "DesiredCount": size
    });
    let target_input_str = target_input.to_string();

    let tz = timezone.unwrap_or("UTC");

    // Check if schedule already exists (properly handles transient errors)
    let exists = schedule_exists(&scheduler, &schedule_name, group_name).await?;

    let target = build_schedule_target(&role_arn, &target_input_str)?;
    let flexible_time_window = build_flexible_time_window()?;

    if exists {
        scheduler
            .update_schedule()
            .name(&schedule_name)
            .group_name(group_name)
            .schedule_expression(schedule_expression)
            .schedule_expression_timezone(tz)
            .flexible_time_window(flexible_time_window)
            .target(target)
            .send()
            .await
            .context("failed to update schedule")?;
    } else {
        scheduler
            .create_schedule()
            .name(&schedule_name)
            .group_name(group_name)
            .schedule_expression(schedule_expression)
            .schedule_expression_timezone(tz)
            .flexible_time_window(flexible_time_window)
            .target(target)
            .send()
            .await
            .context("failed to create schedule")?;
    }

    let action = if exists { "Updated" } else { "Created" };
    println!("✓ Schedule {action}: {schedule_name}");
    println!("  Expression: {schedule_expression} ({tz})");
    println!("  Action:     scale {alias} ({service_name}) to {size}");
    println!("  Group:      {group_name}");
    println!("\n  Use 'oabctl schedule list' to view all schedules");
    println!("  Use 'oabctl schedule delete {schedule_name}' to remove");
    Ok(())
}

/// List all schedules in the oab-schedules group.
pub async fn list_schedules(aws_config: &aws_config::SdkConfig) -> Result<()> {
    let scheduler = aws_sdk_scheduler::Client::new(aws_config);
    let group_name = "oab-schedules";

    let resp = scheduler
        .list_schedules()
        .group_name(group_name)
        .send()
        .await;

    match resp {
        Ok(output) => {
            let schedules = output.schedules();
            if schedules.is_empty() {
                println!("No schedules found in group '{group_name}'.");
                println!("  Use 'oabctl scale <alias> <size> --with-schedule <expr>' to create one.");
                return Ok(());
            }

            println!(
                "{:<40} {:<30} {:<16} STATE",
                "NAME", "SCHEDULE", "TIMEZONE"
            );
            for s in schedules {
                let name = s.name().unwrap_or("-");
                let state = s.state().map(|st| st.as_str()).unwrap_or("?");

                // Fetch full schedule to get expression and timezone
                // TODO: N+1 API calls — consider batching with JoinSet if schedule count grows
                let (expr, tz) = match scheduler
                    .get_schedule()
                    .name(name)
                    .group_name(group_name)
                    .send()
                    .await
                {
                    Ok(detail) => {
                        let e = detail.schedule_expression().unwrap_or("-").to_string();
                        let t = detail
                            .schedule_expression_timezone()
                            .unwrap_or("UTC")
                            .to_string();
                        (e, t)
                    }
                    Err(_) => ("-".to_string(), "-".to_string()),
                };

                println!("{:<40} {:<30} {:<16} {}", name, expr, tz, state);
            }
        }
        Err(e) => {
            if e.as_service_error()
                .map(|se| se.is_resource_not_found_exception())
                .unwrap_or(false)
            {
                println!("No schedules configured yet.");
                println!("  Use 'oabctl scale <alias> <size> --with-schedule <expr>' to create one.");
            } else {
                anyhow::bail!("failed to list schedules: {e}");
            }
        }
    }

    Ok(())
}

/// Delete a specific schedule.
pub async fn delete_schedule(aws_config: &aws_config::SdkConfig, name: &str) -> Result<()> {
    let scheduler = aws_sdk_scheduler::Client::new(aws_config);
    let group_name = "oab-schedules";

    scheduler
        .delete_schedule()
        .name(name)
        .group_name(group_name)
        .send()
        .await
        .context(format!("failed to delete schedule '{name}'"))?;

    println!("✓ Deleted schedule: {name}");
    Ok(())
}

/// Ensure the oab-schedules group exists (idempotent).
async fn ensure_schedule_group(
    scheduler: &aws_sdk_scheduler::Client,
    group_name: &str,
) -> Result<()> {
    let resp = scheduler.get_schedule_group().name(group_name).send().await;

    if resp.is_err() {
        let create_result = scheduler
            .create_schedule_group()
            .name(group_name)
            .send()
            .await;

        // Ignore ConflictException (race condition / already exists)
        if let Err(e) = create_result {
            let err_str = format!("{:?}", e);
            if !err_str.contains("ConflictException") {
                anyhow::bail!("failed to create schedule group: {e}");
            }
        }
    }

    Ok(())
}

/// Ensure the oab-scheduler-role exists (for EventBridge Scheduler to call ECS).
/// Also verifies the inline policy is attached (recovers from partial-state).
async fn ensure_scheduler_role(
    iam: &aws_sdk_iam::Client,
    account_id: &str,
    region: &str,
    cluster: &str,
) -> Result<String> {
    let role_name = "oab-scheduler-role";
    let role_arn = format!("arn:aws:iam::{}:role/{}", account_id, role_name);
    let policy_name = "oab-ecs-scale";

    // Check if role exists
    let role_exists = iam.get_role().role_name(role_name).send().await.is_ok();

    if !role_exists {
        // Create the role with confused-deputy protection
        let trust_policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Principal": {
                    "Service": "scheduler.amazonaws.com"
                },
                "Action": "sts:AssumeRole",
                "Condition": {
                    "StringEquals": {
                        "aws:SourceAccount": account_id
                    }
                }
            }]
        });

        iam.create_role()
            .role_name(role_name)
            .assume_role_policy_document(trust_policy.to_string())
            .description(
                "Allows EventBridge Scheduler to call ECS UpdateService for oabctl scale schedules",
            )
            .send()
            .await
            .context("failed to create scheduler IAM role")?;

        println!("  ✓ Created IAM role: {role_name}");
    }

    // Always ensure inline policy exists (recovers from partial-state)
    let policy_exists = iam
        .get_role_policy()
        .role_name(role_name)
        .policy_name(policy_name)
        .send()
        .await
        .is_ok();

    if !policy_exists {
        let ecs_policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Action": "ecs:UpdateService",
                "Resource": format!("arn:aws:ecs:{region}:{account_id}:service/{cluster}/*")
            }]
        });

        iam.put_role_policy()
            .role_name(role_name)
            .policy_name(policy_name)
            .policy_document(ecs_policy.to_string())
            .send()
            .await
            .context("failed to attach policy to scheduler role")?;

        println!("  ✓ Attached policy: {policy_name}");
    }

    // Wait for IAM propagation if role was just created
    if !role_exists {
        eprintln!("  Waiting for IAM role propagation...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    Ok(role_arn)
}

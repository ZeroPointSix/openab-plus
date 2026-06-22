mod manifest;
mod apply;
mod bootstrap;
mod config;
mod create;
mod get;
mod delete;

use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "oabctl", about = "OAB agent provisioner for ECS")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create or update OAB services from manifest files
    Apply {
        /// Path to manifest file or directory
        #[arg(short, long)]
        file: String,
        /// Skip syncing local config.toml to S3 before applying
        #[arg(long)]
        no_sync: bool,
        /// Wait for deployment to stabilize
        #[arg(long)]
        wait: bool,
    },
    /// Interactive wizard to create a new agent
    Create {
        /// Agent name
        name: String,
        /// Namespace
        #[arg(long, default_value = "prod")]
        namespace: String,
        /// Automatically apply after generating (default: just create files)
        #[arg(long)]
        auto_apply: bool,
    },
    /// List OAB services and their status
    Get {
        /// Resource type
        resource: String,
        /// Optional resource name
        name: Option<String>,
        /// ECS cluster name
        #[arg(long, default_value = "oab")]
        cluster: String,
    },
    /// Delete an OAB service
    Delete {
        /// Resource type
        resource: String,
        /// Resource name
        name: String,
        /// ECS cluster name
        #[arg(long, default_value = "oab")]
        cluster: String,
        /// Namespace
        #[arg(long, default_value = "prod")]
        namespace: String,
    },
    /// Execute a command in an agent container
    Exec {
        /// Agent name
        agent: String,
        /// ECS cluster name
        #[arg(long, default_value = "oab")]
        cluster: String,
        /// Namespace (used in service name: oab-{namespace}-{agent})
        #[arg(long, default_value = "prod")]
        namespace: String,
        /// Command to run (default: /bin/sh). Use -- to separate args.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Copy files to/from agent containers (via ecsctl)
    Cp {
        /// Source path (local or agent:/path)
        src: String,
        /// Destination path (local or agent:/path)
        dst: String,
        /// ECS cluster name
        #[arg(long, default_value = "oab")]
        cluster: String,
        /// Namespace (used in service name: oab-{namespace}-{agent})
        #[arg(long, default_value = "prod")]
        namespace: String,
    },
    /// Sync directories between local machine and agent containers (via ecsctl)
    Sync {
        /// Source: local dir or agent:/path
        src: String,
        /// Destination: agent:/path or local dir
        dst: String,
        /// ECS cluster name
        #[arg(long, default_value = "oab")]
        cluster: String,
        /// Namespace (used in service name: oab-{namespace}-{agent})
        #[arg(long, default_value = "prod")]
        namespace: String,
    },
    /// Bootstrap OAB infrastructure (cluster, IAM roles, S3, security group)
    Bootstrap {
        /// Delete all bootstrap resources
        #[arg(long)]
        delete: bool,
        /// Show current bootstrap status
        #[arg(long)]
        status: bool,
        /// AWS region (defaults to AWS_DEFAULT_REGION or us-east-1)
        #[arg(long)]
        region: Option<String>,
        /// Use existing ECS cluster (skip creation)
        #[arg(long)]
        cluster: Option<String>,
        /// Use existing VPC
        #[arg(long)]
        vpc: Option<String>,
        /// Use existing subnets (comma-separated)
        #[arg(long, value_delimiter = ',')]
        subnets: Option<Vec<String>>,
        /// Use existing security group
        #[arg(long, alias = "sg")]
        security_group: Option<String>,
        /// Use existing task execution role ARN
        #[arg(long)]
        execution_role: Option<String>,
        /// Use existing task role ARN
        #[arg(long)]
        task_role: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

    match cli.command {
        Commands::Apply { file, no_sync, wait } => apply::run(&config, &file, !no_sync, wait).await,
        Commands::Create { name, namespace, auto_apply } => create::run(&config, &name, &namespace, auto_apply).await,
        Commands::Get { resource, name, cluster } => get::run(&config, &resource, name.as_deref(), &cluster).await,
        Commands::Delete { resource, name, cluster, namespace } => {
            delete::run(&config, &resource, &name, &cluster, &namespace).await
        }
        Commands::Exec { agent, cluster, namespace, command } => {
            let resolved = resolve_agent(&config, &agent, &cluster, &namespace).await?;
            let cmd = if command.is_empty() {
                None
            } else {
                let joined = command.iter().map(|a| {
                    format!("'{}'", a.replace('\'', "'\\''"))
                }).collect::<Vec<_>>().join(" ");
                Some(joined)
            };
            ecsctl::exec::run(&config, &resolved, cmd.as_deref()).await
        }
        Commands::Cp { src, dst, cluster, namespace } => {
            let src = resolve_remote_path(&config, &src, &cluster, &namespace).await?;
            let dst = resolve_remote_path(&config, &dst, &cluster, &namespace).await?;
            eprintln!("⇄ Copying {} → {} ...", src, dst);
            ecsctl::cp::run(&config, &src, &dst, None, 60).await?;
            eprintln!("✓ Done");
            Ok(())
        }
        Commands::Sync { src, dst, cluster, namespace } => {
            let src = resolve_remote_path(&config, &src, &cluster, &namespace).await?;
            let dst = resolve_remote_path(&config, &dst, &cluster, &namespace).await?;
            let src_remote = ecsctl::cp::is_remote(&src);
            let dst_remote = ecsctl::cp::is_remote(&dst);
            eprintln!("⇄ Syncing {} → {} ...", src, dst);
            match (src_remote, dst_remote) {
                (false, true) => {
                    ecsctl::sync::run(&config, &src, &dst, None, 60).await?;
                }
                (true, false) => {
                    ecsctl::sync::run_download(&config, &src, &dst, None, 60).await?;
                }
                _ => anyhow::bail!("exactly one of src/dst must be a remote path (agent:/path)"),
            }
            eprintln!("✓ Done");
            Ok(())
        }
        Commands::Bootstrap { delete, status, region, cluster, vpc, subnets, security_group, execution_role, task_role } => {
            let cfg = if let Some(ref r) = region {
                aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(aws_config::Region::new(r.clone()))
                    .load()
                    .await
            } else {
                config.clone()
            };
            let imports = bootstrap::ImportOptions {
                cluster,
                vpc,
                subnets,
                security_group,
                execution_role,
                task_role,
            };
            bootstrap::run(&cfg, delete, status, imports).await
        }
    }
}

/// Resolve agent name to cluster/task_id/container for ECS Exec.
/// Looks up service "{cluster}-{namespace}-{name}" in the given cluster.
async fn resolve_agent(config: &aws_config::SdkConfig, name: &str, cluster: &str, namespace: &str) -> anyhow::Result<String> {
    // If already in cluster/task/container format, pass through
    if name.contains('/') {
        return Ok(name.to_string());
    }

    let ecs = aws_sdk_ecs::Client::new(config);
    let service_name = format!("{cluster}-{namespace}-{name}");

    let tasks = ecs.list_tasks()
        .cluster(cluster)
        .service_name(&service_name)
        .desired_status(aws_sdk_ecs::types::DesiredStatus::Running)
        .send().await
        .context(format!("failed to list tasks for {service_name}"))?;

    let task_arn = tasks.task_arns().first()
        .context(format!("no running tasks for agent '{name}'"))?;

    let task_id = task_arn.rsplit('/').next().unwrap_or(task_arn);
    Ok(format!("{cluster}/{task_id}/openab"))
}

/// Resolve remote path "agent:/path" using dynamic ECS task lookup.
/// Local paths are returned unchanged.
async fn resolve_remote_path(config: &aws_config::SdkConfig, path: &str, cluster: &str, namespace: &str) -> anyhow::Result<String> {
    if !path.contains(':') {
        return Ok(path.to_string());
    }
    let (agent, remote_path) = path.split_once(':')
        .context("invalid remote path format")?;
    let resolved = resolve_agent(config, agent, cluster, namespace).await?;
    Ok(format!("{resolved}:{remote_path}"))
}

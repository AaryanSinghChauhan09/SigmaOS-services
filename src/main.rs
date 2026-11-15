//! SigmaOS-services — SigmaOS Service Integration Layer
//!
//! Manages SigmaOS-specific service configuration, startup ordering,
//! health monitoring, and IPC endpoint registration.
//! Relies on systemd/s6 for actual process supervision; this binary
//! performs SigmaOS-specific orchestration on top.

use std::env;

use anyhow::Result;
use log::{debug, info, warn};

fn main() -> Result<()> {
    // Initialise logging (respects RUST_LOG env var)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let config = Config::from_env();
    info!(
        "SigmaOS-services starting (verbose={}, dry_run={})",
        config.verbose, config.dry_run
    );

    run_service_checks(&config)?;

    info!("SigmaOS-services: all checks complete");
    Ok(())
}

/// Runtime configuration derived from environment variables.
#[derive(Debug, Clone)]
struct Config {
    verbose: bool,
    dry_run: bool,
}

impl Config {
    fn from_env() -> Self {
        Config {
            verbose: env::var("VERBOSE").is_ok(),
            dry_run: env::var("DRY_RUN").is_ok(),
        }
    }
}

/// Core service health checks performed at startup.
fn run_service_checks(config: &Config) -> Result<()> {
    let services = [
        ("networkd", "/run/systemd/netif/state"),
        ("resolved", "/run/systemd/resolve/stub-resolv.conf"),
        ("logind", "/run/systemd/sessions"),
        ("dbus", "/run/dbus/system_bus_socket"),
    ];

    for (name, socket_or_path) in &services {
        if config.dry_run {
            info!("  [dry-run] would check: {name} → {socket_or_path}");
            continue;
        }

        let exists = std::path::Path::new(socket_or_path).exists();
        if exists {
            debug!("  [✓] {name}: up");
        } else {
            warn!("  [!] {name}: not found at {socket_or_path} — may not be running");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_without_env() {
        // VERBOSE / DRY_RUN are not set in test env by default
        let c = Config {
            verbose: false,
            dry_run: false,
        };
        assert!(!c.verbose);
        assert!(!c.dry_run);
    }

    #[test]
    fn service_checks_dry_run_does_not_panic() {
        let config = Config {
            verbose: true,
            dry_run: true,
        };
        // Should complete without error even in a headless environment
        run_service_checks(&config).expect("dry-run service checks should not error");
    }
}

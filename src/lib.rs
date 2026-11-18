// SPDX-License-Identifier: GPL-3.0-or-later
//! SigmaOS-services — Unified system service supervisor.
//!
//! Manages the lifecycle, health checks, dependency ordering, and restart
//! policies for core system services: network, audio, Bluetooth, power,
//! authentication, storage, and backup.
//!
//! Inspired by systemd service units / OpenRC runlevels / s6 supervision.
//! Principles: fail-fast, no unsafe code, deterministic unit-testable logic.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    EmptyName,
    ServiceNotFound(String),
    ServiceAlreadyExists(String),
    CyclicDependency(String),
    DependencyFailed { service: String, dependency: String },
    AlreadyRunning(String),
    NotRunning(String),
    MaxRestartsExceeded(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "service name cannot be empty"),
            Self::ServiceNotFound(s) => write!(f, "service '{s}' not found"),
            Self::ServiceAlreadyExists(s) => write!(f, "service '{s}' already registered"),
            Self::CyclicDependency(s) => write!(f, "cyclic dependency detected involving '{s}'"),
            Self::DependencyFailed { service, dependency } => {
                write!(f, "cannot start '{service}': dependency '{dependency}' is not running")
            }
            Self::AlreadyRunning(s) => write!(f, "service '{s}' is already running"),
            Self::NotRunning(s) => write!(f, "service '{s}' is not running"),
            Self::MaxRestartsExceeded(s) => write!(f, "service '{s}' exceeded max restart attempts"),
        }
    }
}

impl std::error::Error for ServiceError {}

// ── Service Category ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceCategory {
    Network,
    Audio,
    Bluetooth,
    Power,
    Authentication,
    Storage,
    Backup,
    Custom,
}

impl fmt::Display for ServiceCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Network => "network",
            Self::Audio => "audio",
            Self::Bluetooth => "bluetooth",
            Self::Power => "power",
            Self::Authentication => "authentication",
            Self::Storage => "storage",
            Self::Backup => "backup",
            Self::Custom => "custom",
        })
    }
}

// ── Service State ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    Stopped,
    Starting,
    Running { pid: u32, uptime_secs: u64 },
    Degraded { reason: String },
    Failed { error: String },
    Stopping,
}

impl ServiceState {
    pub fn is_active(&self) -> bool {
        matches!(self, ServiceState::Running { .. })
    }
}

// ── Restart Policy ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    No,
    Always,
    OnFailure { max_retries: u32 },
}

// ── Service Definition ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ServiceUnit {
    pub name: String,
    pub category: ServiceCategory,
    pub description: String,
    pub dependencies: Vec<String>,
    pub restart_policy: RestartPolicy,
    pub restart_count: u32,
    pub state: ServiceState,
}

impl ServiceUnit {
    pub fn new(
        name: impl Into<String>,
        category: ServiceCategory,
        description: impl Into<String>,
    ) -> Result<Self, ServiceError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ServiceError::EmptyName);
        }
        Ok(Self {
            name,
            category,
            description: description.into(),
            dependencies: Vec::new(),
            restart_policy: RestartPolicy::OnFailure { max_retries: 3 },
            restart_count: 0,
            state: ServiceState::Stopped,
        })
    }

    pub fn with_dependency(mut self, dep: impl Into<String>) -> Self {
        self.dependencies.push(dep.into());
        self
    }

    pub fn with_restart_policy(mut self, policy: RestartPolicy) -> Self {
        self.restart_policy = policy;
        self
    }
}

// ── Supervisor ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ServiceSupervisor {
    units: HashMap<String, ServiceUnit>,
    next_pid: u32,
}

impl ServiceSupervisor {
    pub fn new() -> Self {
        Self {
            units: HashMap::new(),
            next_pid: 1000,
        }
    }

    /// Register a service unit. Fails if name is duplicate.
    pub fn register(&mut self, unit: ServiceUnit) -> Result<(), ServiceError> {
        if self.units.contains_key(&unit.name) {
            return Err(ServiceError::ServiceAlreadyExists(unit.name));
        }
        self.units.insert(unit.name.clone(), unit);
        Ok(())
    }

    /// Get service reference.
    pub fn get(&self, name: &str) -> Option<&ServiceUnit> {
        self.units.get(name)
    }

    /// Get mutable service reference.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut ServiceUnit> {
        self.units.get_mut(name)
    }

    /// Total registered service count.
    pub fn count(&self) -> usize {
        self.units.len()
    }

    /// Count of currently active (running) services.
    pub fn active_count(&self) -> usize {
        self.units.values().filter(|u| u.state.is_active()).count()
    }

    /// Check for cyclic dependencies in registered services.
    pub fn validate_dependencies(&self) -> Result<(), ServiceError> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for name in self.units.keys() {
            if !visited.contains(name.as_str()) {
                self.check_cycle(name.as_str(), &mut visited, &mut rec_stack)?;
            }
        }
        Ok(())
    }

    fn check_cycle<'a>(
        &'a self,
        current: &'a str,
        visited: &mut HashSet<&'a str>,
        rec_stack: &mut HashSet<&'a str>,
    ) -> Result<(), ServiceError> {
        visited.insert(current);
        rec_stack.insert(current);

        if let Some(unit) = self.units.get(current) {
            for dep in &unit.dependencies {
                if !visited.contains(dep.as_str()) {
                    self.check_cycle(dep, visited, rec_stack)?;
                } else if rec_stack.contains(dep.as_str()) {
                    return Err(ServiceError::CyclicDependency(current.to_string()));
                }
            }
        }

        rec_stack.remove(current);
        Ok(())
    }

    /// Start a service unit, checking that all dependencies are active first.
    pub fn start(&mut self, name: &str) -> Result<(), ServiceError> {
        let unit = self.units.get(name).ok_or_else(|| ServiceError::ServiceNotFound(name.to_string()))?;

        if unit.state.is_active() {
            return Err(ServiceError::AlreadyRunning(name.to_string()));
        }

        // Verify all dependencies are running
        for dep in &unit.dependencies {
            let dep_unit = self.units.get(dep).ok_or_else(|| ServiceError::ServiceNotFound(dep.clone()))?;
            if !dep_unit.state.is_active() {
                return Err(ServiceError::DependencyFailed {
                    service: name.to_string(),
                    dependency: dep.clone(),
                });
            }
        }

        self.next_pid += 1;
        let pid = self.next_pid;

        let unit_mut = self.units.get_mut(name).unwrap();
        unit_mut.state = ServiceState::Running {
            pid,
            uptime_secs: 0,
        };
        Ok(())
    }

    /// Stop a running service unit.
    pub fn stop(&mut self, name: &str) -> Result<(), ServiceError> {
        let unit = self.units.get_mut(name).ok_or_else(|| ServiceError::ServiceNotFound(name.to_string()))?;

        if !unit.state.is_active() {
            return Err(ServiceError::NotRunning(name.to_string()));
        }

        unit.state = ServiceState::Stopped;
        Ok(())
    }

    /// Trigger a failure on a service and apply restart policy.
    pub fn handle_failure(&mut self, name: &str, error: &str) -> Result<bool, ServiceError> {
        let unit = self.units.get_mut(name).ok_or_else(|| ServiceError::ServiceNotFound(name.to_string()))?;

        unit.state = ServiceState::Failed { error: error.to_string() };

        match unit.restart_policy {
            RestartPolicy::No => Ok(false),
            RestartPolicy::Always => {
                unit.restart_count += 1;
                unit.state = ServiceState::Running { pid: 9999, uptime_secs: 0 };
                Ok(true)
            }
            RestartPolicy::OnFailure { max_retries } => {
                if unit.restart_count < max_retries {
                    unit.restart_count += 1;
                    unit.state = ServiceState::Running { pid: 9999, uptime_secs: 0 };
                    Ok(true)
                } else {
                    Err(ServiceError::MaxRestartsExceeded(name.to_string()))
                }
            }
        }
    }

    /// Compute topologically sorted boot order for all services.
    pub fn startup_order(&self) -> Result<Vec<String>, ServiceError> {
        self.validate_dependencies()?;

        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

        for (name, unit) in &self.units {
            in_degree.entry(name.as_str()).or_insert(0);
            for dep in &unit.dependencies {
                adj.entry(dep.as_str()).or_default().push(name.as_str());
                *in_degree.entry(name.as_str()).or_insert(0) += 1;
            }
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&name, _)| name)
            .collect();

        let mut order = Vec::new();
        while let Some(u) = queue.pop_front() {
            order.push(u.to_string());
            if let Some(neighbors) = adj.get(u) {
                for &v in neighbors {
                    if let Some(deg) = in_degree.get_mut(v) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(v);
                        }
                    }
                }
            }
        }

        Ok(order)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_service_unit() {
        let u = ServiceUnit::new("sig-net", ServiceCategory::Network, "Network daemon").unwrap();
        assert_eq!(u.name, "sig-net");
        assert_eq!(u.category, ServiceCategory::Network);
        assert_eq!(u.state, ServiceState::Stopped);
    }

    #[test]
    fn test_empty_service_name_fails() {
        assert!(ServiceUnit::new("  ", ServiceCategory::Audio, "test").is_err());
    }

    #[test]
    fn test_duplicate_registration_fails() {
        let mut sup = ServiceSupervisor::new();
        let u1 = ServiceUnit::new("sig-audio", ServiceCategory::Audio, "Audio").unwrap();
        let u2 = ServiceUnit::new("sig-audio", ServiceCategory::Audio, "Audio2").unwrap();
        sup.register(u1).unwrap();
        assert!(matches!(sup.register(u2), Err(ServiceError::ServiceAlreadyExists(_))));
    }

    #[test]
    fn test_start_stop_service() {
        let mut sup = ServiceSupervisor::new();
        let u = ServiceUnit::new("sig-power", ServiceCategory::Power, "Power management").unwrap();
        sup.register(u).unwrap();
        assert_eq!(sup.active_count(), 0);

        sup.start("sig-power").unwrap();
        assert_eq!(sup.active_count(), 1);
        assert!(sup.get("sig-power").unwrap().state.is_active());

        sup.stop("sig-power").unwrap();
        assert_eq!(sup.active_count(), 0);
    }

    #[test]
    fn test_start_unmet_dependency_fails() {
        let mut sup = ServiceSupervisor::new();
        let net = ServiceUnit::new("net", ServiceCategory::Network, "Net").unwrap();
        let auth = ServiceUnit::new("auth", ServiceCategory::Authentication, "Auth")
            .unwrap()
            .with_dependency("net");

        sup.register(net).unwrap();
        sup.register(auth).unwrap();

        // Starting auth before net should fail
        assert!(matches!(
            sup.start("auth"),
            Err(ServiceError::DependencyFailed { .. })
        ));

        // Start net first, then auth succeeds
        sup.start("net").unwrap();
        assert!(sup.start("auth").is_ok());
    }

    #[test]
    fn test_cyclic_dependency_detection() {
        let mut sup = ServiceSupervisor::new();
        let a = ServiceUnit::new("a", ServiceCategory::Custom, "A")
            .unwrap()
            .with_dependency("b");
        let b = ServiceUnit::new("b", ServiceCategory::Custom, "B")
            .unwrap()
            .with_dependency("a");

        sup.register(a).unwrap();
        sup.register(b).unwrap();

        assert!(matches!(
            sup.validate_dependencies(),
            Err(ServiceError::CyclicDependency(_))
        ));
    }

    #[test]
    fn test_startup_order() {
        let mut sup = ServiceSupervisor::new();
        let net = ServiceUnit::new("net", ServiceCategory::Network, "Net").unwrap();
        let storage = ServiceUnit::new("storage", ServiceCategory::Storage, "Storage").unwrap();
        let backup = ServiceUnit::new("backup", ServiceCategory::Backup, "Backup")
            .unwrap()
            .with_dependency("storage")
            .with_dependency("net");

        sup.register(net).unwrap();
        sup.register(storage).unwrap();
        sup.register(backup).unwrap();

        let order = sup.startup_order().unwrap();
        let backup_pos = order.iter().position(|x| x == "backup").unwrap();
        let storage_pos = order.iter().position(|x| x == "storage").unwrap();
        let net_pos = order.iter().position(|x| x == "net").unwrap();

        assert!(storage_pos < backup_pos);
        assert!(net_pos < backup_pos);
    }

    #[test]
    fn test_restart_policy_on_failure() {
        let mut sup = ServiceSupervisor::new();
        let u = ServiceUnit::new("flaky", ServiceCategory::Custom, "Flaky")
            .unwrap()
            .with_restart_policy(RestartPolicy::OnFailure { max_retries: 2 });
        sup.register(u).unwrap();

        assert_eq!(sup.handle_failure("flaky", "crash1").unwrap(), true);
        assert_eq!(sup.handle_failure("flaky", "crash2").unwrap(), true);
        assert!(matches!(
            sup.handle_failure("flaky", "crash3"),
            Err(ServiceError::MaxRestartsExceeded(_))
        ));
    }
}

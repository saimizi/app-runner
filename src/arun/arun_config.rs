#[allow(unused)]
use {
    arunlib::{arun_error::ArunError, utils::IntervalTimer},
    bollard::models::DeviceMapping,
    error_stack::{IntoReport, Report, Result, ResultExt},
    jlogger_tracing::{
        jdebug, jerror, jinfo, jtrace, jwarn, JloggerBuilder, LevelFilter, LogTimeFormat,
    },
    serde::{Deserialize, Serialize},
    serde_json,
    std::{fmt::Display, ops::Deref},
};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum AppType {
    Sys,
    User,
}

impl Display for AppType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            AppType::Sys => "sys",
            AppType::User => "user",
        };

        write!(f, "{}", output)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArunDeviceMapping {
    path_on_host: String,
    path_in_container: String,
    cgroup_permissions: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PortBindingInfo {
    pub port: String,
    pub host: Vec<String>,
}

impl Deref for PortBindingInfo {
    type Target = PortBindingInfo;

    fn deref(&self) -> &Self::Target {
        self
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArunConfig {
    name: String,
    app_type: AppType,
    image: String,
    version: String,
    privilege: bool,
    network: String,
    port_bindings: Option<Vec<PortBindingInfo>>,
    cmd: String,
    binds: Vec<String>,
    features: Vec<String>,
    environments: Vec<String>,
    monitor_interval: Option<u32>,
}

impl Default for ArunConfig {
    fn default() -> Self {
        Self {
            name: "Invalid".to_string(),
            app_type: AppType::User,
            image: "Invalid".to_string(),
            version: "latest".to_string(),
            privilege: false,
            network: "none".to_string(),
            port_bindings: None,
            cmd: "Invalid".to_string(),
            binds: Vec::new(),
            features: Vec::new(),
            environments: Vec::new(),
            monitor_interval: Some(1_u32),
        }
    }
}

impl ArunConfig {
    pub fn parse(json: &str, monitor_interval_s: Option<u32>) -> Result<ArunConfig, ArunError> {
        let mut config: ArunConfig = serde_json::from_str(json)
            .into_report()
            .change_context(ArunError::InvalidValue)
            .attach_printable(format!("Failed to parse json string:\n {}", json))?;

        if let Some(m) = monitor_interval_s {
            config.monitor_interval = Some(m);
        }

        if config.monitor_interval.is_none() {
            config.monitor_interval = Some(1_u32);
        }

        Ok(config)
    }

    pub fn image(&self) -> String {
        format!("{}:{}", self.image, self.version)
    }

    pub fn image_name(&self) -> &str {
        &self.image
    }

    pub fn image_version(&self) -> &str {
        &self.version
    }

    pub fn appid(&self) -> String {
        format!("{}.{}", self.app_type, self.name)
    }

    pub fn network(&self) -> &str {
        jdebug!("network: {}", self.network);
        &self.network
    }

    pub fn cmd(&self) -> Vec<String> {
        let mut cmd: Vec<String> = self
            .cmd
            .as_str()
            .rsplit(' ')
            .into_iter()
            .map(|a| a.to_string())
            .collect();

        cmd.reverse();
        cmd
    }

    pub fn environment(&self) -> Vec<String> {
        self.environments.clone()
    }

    pub fn privilege(&self) -> bool {
        self.privilege
    }

    pub fn binds(&self) -> Vec<&str> {
        self.binds.iter().map(|s| s.as_str()).collect()
    }

    pub fn gui(&self) -> bool {
        self.features.iter().any(|f| f.as_str() == "gui")
    }

    pub fn redis_server(&self) -> bool {
        self.features.iter().any(|f| f.as_str() == "redis-server")
    }

    pub fn wayland(&self) -> bool {
        self.features.iter().any(|f| f.as_str() == "wayland")
    }

    pub fn port_bindings(&self) -> Option<&[PortBindingInfo]> {
        self.port_bindings.as_deref()
    }

    pub fn monitor_interval(&self) -> u32 {
        self.monitor_interval.unwrap()
    }
}

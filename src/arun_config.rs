#[allow(unused)]
use {
    super::arun_error::ArunError,
    bollard::models::DeviceMapping,
    error_stack::{IntoReport, Report, Result, ResultExt},
    jlogger_tracing::{
        jdebug, jerror, jinfo, jtrace, jwarn, JloggerBuilder, LevelFilter, LogTimeFormat,
    },
    serde::{Deserialize, Serialize},
    serde_json,
    std::fmt::Display,
};

#[allow(non_camel_case_types)]
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum NetworkType {
    none,
    host,
    container,
}

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
pub struct ArunConfig {
    name: String,
    app_type: AppType,
    image: String,
    version: String,
    privilege: bool,
    network: NetworkType,
    cmd: String,
    gui: bool,
    binds: Vec<String>,
    devices: Vec<ArunDeviceMapping>,
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
            network: NetworkType::none,
            cmd: "Invalid".to_string(),
            gui: false,
            binds: Vec::new(),
            devices: Vec::new(),
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

    pub fn appid(&self) -> String {
        format!("{}.{}", self.app_type, self.name)
    }

    pub fn network(&self) -> NetworkType {
        self.network
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

    pub fn binds(&self) -> Vec<String> {
        self.binds.clone()
    }

    pub fn environment(&self) -> Vec<String> {
        self.environments.clone()
    }

    pub fn privilege(&self) -> bool {
        self.privilege
    }

    pub fn gui(&self) -> bool {
        self.gui
    }

    pub fn devices(&self) -> Vec<DeviceMapping> {
        let mut result = Vec::new();

        for d in &self.devices {
            result.push(DeviceMapping {
                path_on_host: Some(d.path_on_host.clone()),
                path_in_container: Some(d.path_in_container.clone()),
                cgroup_permissions: Some(d.cgroup_permissions.clone()),
            });
        }

        result
    }

    pub fn monitor_interval(&self) -> u32 {
        self.monitor_interval.unwrap()
    }
}

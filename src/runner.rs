#[allow(unused)]
use {
    bollard::{
        container, image,
        models::{DeviceMapping, HostConfig},
        Docker,
    },
    error_stack::{IntoReport, Report, Result, ResultExt},
    futures::StreamExt,
    jlogger_tracing::{
        jdebug, jerror, jinfo, jtrace, jwarn, JloggerBuilder, LevelFilter, LogTimeFormat,
    },
    serde::{Deserialize, Serialize},
    serde_json,
};

use std::{collections::HashMap, fmt::Display, str::FromStr};

use super::{
    arun_config::{ArunConfig, NetworkType},
    arun_error::ArunError,
    utils::IntervalTimer,
};

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum RunnerState {
    NonExist,
    Created,
    Running,
    Restarting,
    Exited,
    Paused,
    Dead,
}

impl Display for RunnerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_str = match self {
            RunnerState::NonExist => "NonExist",
            RunnerState::Created => "Created",
            RunnerState::Running => "Running",
            RunnerState::Restarting => "Restarting",
            RunnerState::Exited => "Exited",
            RunnerState::Dead => "Dead",
            RunnerState::Paused => "Paused",
        };

        write!(f, "{}", state_str)
    }
}

impl FromStr for RunnerState {
    type Err = ArunError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "created" => Ok(RunnerState::Created),
            "running" => Ok(RunnerState::Running),
            "restarting" => Ok(RunnerState::Restarting),
            "exited" => Ok(RunnerState::Exited),
            "paused" => Ok(RunnerState::Paused),
            "dead" => Ok(RunnerState::Dead),
            "nonExist" => Ok(RunnerState::NonExist),
            _ => Err(ArunError::Unknown),
        }
    }
}

pub struct Runner {
    state: RunnerState,
    docker: Docker,
    config: ArunConfig,
}

impl Runner {
    pub fn host_config(arun_config: &ArunConfig) -> HostConfig {
        let mut device_mapping = vec![];

        let mut binds: Vec<String> = arun_config.binds().iter().map(|s| s.to_string()).collect();

        // A privileged container dose not need a specific device mapping.
        if !arun_config.privilege() && arun_config.gui() {
            let drm_device = vec!["/dev/dri/card0", "/dev/dri/card1"];

            drm_device.iter().for_each(|d| {
                device_mapping.push(DeviceMapping {
                    path_on_host: Some(d.to_string()),
                    path_in_container: Some(d.to_string()),
                    cgroup_permissions: Some("rwm".to_string()),
                });
            });
        }

        if arun_config.wayland() {
            binds.push("/run/user/0:/run/user/0:rw".to_owned());
        }

        HostConfig {
            binds: Some(binds),
            privileged: Some(arun_config.privilege()),
            devices: Some(device_mapping),
            ..Default::default()
        }
    }

    async fn update_state(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();

        let mut filters = HashMap::new();
        filters.insert("name", vec![container_name.as_str()]);

        let options = container::ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let summary = self
            .docker
            .list_containers(Some(options))
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let mut state = RunnerState::NonExist;

        if summary.is_empty() {
            jdebug!("No container found for {}", container_name);
        } else {
            jdebug!("found container with name of {}", container_name);
        }

        for c in summary {
            let image = c.image.ok_or(ArunError::DockerErr).into_report()?;
            if image == self.config.image() {
                let s = c.state.ok_or(ArunError::DockerErr).into_report()?;
                let new_state = RunnerState::from_str(s.as_str())
                    .into_report()
                    .attach_printable(format!("Failed to recognize state {}", s))?;

                if state != RunnerState::NonExist && state != new_state {
                    return Err(ArunError::Unknown)
                        .into_report()
                        .attach_printable(format!(
                            "Two state {} vs {} found for container with name {}.",
                            state, new_state, container_name
                        ));
                }

                state = new_state;
                continue;
            }

            return Err(ArunError::DockerErr)
                .into_report()
                .change_context(ArunError::DockerErr)
                .attach_printable(format!(
                    "Another container with image {} is running with name {}",
                    image, container_name
                ));
        }

        self.state = state;
        Ok(())
    }

    pub async fn new(json: &str, monitor_interval: Option<u32>) -> Result<Self, ArunError> {
        let arun_config = ArunConfig::parse(json, monitor_interval)?;
        jdebug!("Arun Config:\n{:?}", arun_config);

        let app = Docker::connect_with_socket_defaults()
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let mut runner = Runner {
            config: arun_config,
            docker: app,
            state: RunnerState::NonExist,
        };

        runner.update_state().await?;
        jdebug!(InitialContainerState = runner.state.to_string());

        Ok(runner)
    }

    pub async fn run(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();

        if self.state == RunnerState::Dead || self.state == RunnerState::Exited {
            let options = container::RemoveContainerOptions {
                v: true,
                force: true,
                link: false,
            };

            self.docker
                .remove_container(container_name.as_str(), Some(options))
                .await
                .into_report()
                .change_context(ArunError::DockerErr)
                .attach_printable(format!(
                    "Failed to remove the previous exited container {}",
                    container_name
                ))?;

            self.state = RunnerState::NonExist;
        }

        if self.state == RunnerState::NonExist {
            let option = container::CreateContainerOptions {
                name: self.config.appid(),
                platform: None,
            };

            let mut env = self.config.environment();
            if self.config.wayland() {
                env.push("XDG_RUNTIME_DIR=/run/user/0".to_owned());
            }

            let config = container::Config {
                image: Some(self.config.image()),
                cmd: Some(self.config.cmd()),
                env: Some(env),
                host_config: Some(Runner::host_config(&self.config)),
                network_disabled: Some(self.config.network() == NetworkType::none),
                ..Default::default()
            };

            self.docker
                .create_container(Some(option), config)
                .await
                .into_report()
                .change_context(ArunError::DockerErr)?;

            self.state = RunnerState::Created;
        }

        if self.state == RunnerState::Created {
            self.docker
                .start_container::<String>(&container_name, None)
                .await
                .into_report()
                .change_context(ArunError::DockerErr)?;
            self.state = RunnerState::Running;
        }

        if self.state == RunnerState::Paused {
            self.docker
                .unpause_container(container_name.as_str())
                .await
                .into_report()
                .change_context(ArunError::DockerErr)
                .attach_printable(format!(
                    "Failed to unpause the container {}",
                    container_name
                ))?;
            self.state = RunnerState::Running;
        }

        let options = container::WaitContainerOptions {
            condition: "not-running",
        };

        let mut wait_stopped = self.docker.wait_container(&container_name, Some(options));

        let options = container::WaitContainerOptions {
            condition: "removed",
        };

        let mut wait_removed = self.docker.wait_container(&container_name, Some(options));

        let options = container::AttachContainerOptions::<String> {
            stdout: Some(true),
            stream: Some(true),
            stderr: Some(true),
            logs: Some(true),
            ..Default::default()
        };

        let container::AttachContainerResults { mut output, input } = self
            .docker
            .attach_container(&container_name, Some(options))
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let _ = input;

        let mut itimer = IntervalTimer::new(tokio::time::Duration::from_secs(
            self.config.monitor_interval() as u64,
        ));
        let mut old_state = self.state;

        let e = loop {
            tokio::select! {
                Some(ret) = wait_stopped.next() => {
                    match ret {
                        Ok(_) => jinfo!(app=container_name, state="stopped"),
                        Err(e) => break e,
                    }
                },

                Some(Ok(log)) = output.next() => {
                    jinfo!("{}", log.to_string().trim());
                },

                Some(ret) = wait_removed.next() => {
                    match ret {
                        Ok(_) => jinfo!(app=container_name, state="removed"),
                        Err(e) => break e,
                    }
                },

                _ = itimer.wait_timeup() => {
                    self.update_state().await?;
                    if self.state != old_state {
                        jinfo!(NewContainerState = self.state.to_string(),OldContainerState = old_state.to_string());
                        old_state = self.state;
                    }

                }
            }
        };

        Err(ArunError::DockerErr)
            .into_report()
            .attach_printable(format!("failed to wait container {:?}", e))
    }
}

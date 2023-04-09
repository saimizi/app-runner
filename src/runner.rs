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
};

#[derive(PartialEq, Eq, Debug)]
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

#[derive(PartialEq, Eq, Debug)]
pub enum PreCheckResult {
    NormalState(RunnerState),
    Conflicted,
}

pub struct Runner {
    state: RunnerState,
    docker: Docker,
    config: ArunConfig,
}

impl Runner {
    pub fn host_config(arun_config: &ArunConfig) -> HostConfig {
        let binds = arun_config.binds();
        let devices = arun_config.devices();

        HostConfig {
            binds: if binds.is_empty() { None } else { Some(binds) },
            privileged: Some(arun_config.privilege()),
            devices: if devices.is_empty() {
                None
            } else {
                Some(devices)
            },
            ..Default::default()
        }
    }

    async fn pre_check(docker: &Docker, config: &ArunConfig) -> Result<PreCheckResult, ArunError> {
        let container_name = config.appid();

        let mut filters = HashMap::new();
        filters.insert("name", vec![container_name.as_str()]);

        let options = container::ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let summary = docker
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
            if image == config.image() {
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

            jwarn!(
                "Another container with image {} is running with name {}",
                image,
                container_name
            );

            return Ok(PreCheckResult::Conflicted);
        }

        Ok(PreCheckResult::NormalState(state))
    }

    pub async fn new(json: &str) -> Result<Self, ArunError> {
        let arun_config = ArunConfig::parse(json)?;
        jdebug!("Arun Config:\n{:?}", arun_config);

        let app = Docker::connect_with_socket_defaults()
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let pre_check_result = Runner::pre_check(&app, &arun_config).await?;
        let state = match pre_check_result {
            PreCheckResult::NormalState(s) => s,
            PreCheckResult::Conflicted => {
                return Err(ArunError::ConflictedWithOther).into_report();
            }
        };

        jdebug!(PreCheckState = state.to_string());

        Ok(Self {
            config: arun_config,
            docker: app,
            state,
        })
    }

    pub async fn run(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();

        if self.state == RunnerState::Dead {
            let options = container::RemoveContainerOptions {
                v: true,
                force: true,
                link: true,
            };

            self.docker
                .remove_container(container_name.as_str(), Some(options))
                .await
                .into_report()
                .change_context(ArunError::DockerErr)
                .attach_printable(format!(
                    "Failed to remove the dead container {}",
                    container_name
                ))?;

            self.state = RunnerState::NonExist;
        }

        if self.state == RunnerState::NonExist {
            let option = container::CreateContainerOptions {
                name: self.config.appid(),
                platform: None,
            };

            let config = container::Config {
                image: Some(self.config.image()),
                cmd: Some(self.config.cmd()),
                env: Some(self.config.environment()),
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

        if self.state == RunnerState::Created || self.state == RunnerState::Exited {
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

            }
        };

        Err(ArunError::DockerErr)
            .into_report()
            .attach_printable(format!("failed to wait container {:?}", e))
    }
}

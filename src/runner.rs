#[allow(unused)]
use {
    super::{
        arun_config::{ArunConfig, NetworkType},
        arun_error::ArunError,
        ctlif::{ArunCtrl, ArunCtrlCmd},
        utils::IntervalTimer,
    },
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
    std::{collections::HashMap, fmt::Display, str::FromStr},
    tokio::sync::mpsc,
};

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum RunnerRequest {
    UpdateState,
}

impl Display for RunnerRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let req_str = match self {
            RunnerRequest::UpdateState => "RunnerRequest::UpdateState",
        };

        write!(f, "{}", req_str)
    }
}

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
    target_state: RunnerState,
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
            target_state: RunnerState::NonExist,
        };

        runner.update_state().await?;
        jdebug!(InitialContainerState = runner.state.to_string());

        Ok(runner)
    }

    pub async fn create(&mut self) -> Result<(), ArunError> {
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

        Ok(())
    }

    pub async fn start(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();

        self.docker
            .start_container::<String>(&container_name, None)
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        self.state = RunnerState::Running;
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();
        let options = container::StopContainerOptions { t: 1_i64 };

        self.docker
            .stop_container(&container_name, Some(options))
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        self.state = RunnerState::Exited;
        Ok(())
    }

    pub async fn pause(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();

        self.docker
            .pause_container(container_name.as_str())
            .await
            .into_report()
            .change_context(ArunError::DockerErr)
            .attach_printable(format!(
                "Failed to unpause the container {}",
                container_name
            ))?;

        self.state = RunnerState::Paused;
        Ok(())
    }

    pub async fn unpause(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();

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
        Ok(())
    }

    pub async fn remove(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();
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

        Ok(())
    }

    pub async fn state_transition(&mut self, target: RunnerState) -> Result<(), ArunError> {
        self.update_state().await?;
        if self.state == target {
            return Ok(());
        }

        if target == RunnerState::NonExist {
            match self.state {
                RunnerState::NonExist => {}
                RunnerState::Created => {
                    self.remove().await?;
                }
                RunnerState::Running => {
                    self.stop().await?;
                    self.remove().await?;
                }
                RunnerState::Restarting => {
                    self.stop().await?;
                    self.remove().await?;
                }
                RunnerState::Exited => {
                    self.remove().await?;
                }
                RunnerState::Paused => {
                    self.remove().await?;
                }
                RunnerState::Dead => {
                    self.remove().await?;
                }
            }
        }

        if target == RunnerState::Created {
            match self.state {
                RunnerState::NonExist => {
                    self.create().await?;
                }
                RunnerState::Created => {}
                RunnerState::Running => {
                    self.stop().await?;
                }
                RunnerState::Restarting => {
                    self.stop().await?;
                }
                RunnerState::Exited => {
                    self.remove().await?;
                    self.create().await?;
                }
                RunnerState::Paused => {
                    self.remove().await?;
                    self.create().await?;
                }
                RunnerState::Dead => {
                    self.create().await?;
                }
            }
        }

        if target == RunnerState::Running {
            match self.state {
                RunnerState::NonExist => {
                    self.create().await?;
                    self.start().await?;
                }
                RunnerState::Created => {
                    self.start().await?;
                }
                RunnerState::Running => {}
                RunnerState::Restarting => {}
                RunnerState::Exited => {
                    self.remove().await?;
                    self.create().await?;
                    self.start().await?;
                }
                RunnerState::Paused => {
                    self.unpause().await?;
                }
                RunnerState::Dead => {
                    self.remove().await?;
                    self.create().await?;
                    self.start().await?;
                }
            }
        }

        if target == RunnerState::Restarting {}

        if target == RunnerState::Exited {
            match self.state {
                RunnerState::NonExist => {
                    self.create().await?;
                    self.start().await?;
                    self.stop().await?;
                }
                RunnerState::Created => {
                    self.start().await?;
                    self.stop().await?;
                }
                RunnerState::Running => {
                    self.stop().await?;
                }
                RunnerState::Restarting => {
                    self.stop().await?;
                }
                RunnerState::Exited => {}
                RunnerState::Paused => {
                    self.stop().await?;
                }
                RunnerState::Dead => {
                    self.remove().await?;
                    self.create().await?;
                    self.start().await?;
                    self.stop().await?;
                }
            }
        }

        if target == RunnerState::Paused {
            match self.state {
                RunnerState::NonExist => {
                    self.create().await?;
                    self.start().await?;
                    self.pause().await?;
                }
                RunnerState::Created => {
                    self.start().await?;
                    self.pause().await?;
                }
                RunnerState::Running => {
                    self.pause().await?;
                }
                RunnerState::Restarting => {
                    self.pause().await?;
                }
                RunnerState::Exited => {
                    self.start().await?;
                    self.pause().await?;
                }
                RunnerState::Paused => {}
                RunnerState::Dead => {
                    self.remove().await?;
                    self.create().await?;
                    self.start().await?;
                    self.pause().await?;
                }
            }
        }

        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), ArunError> {
        let container_name = self.config.appid();

        // Set initial target state to Running
        self.target_state = RunnerState::Running;

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
        let mut ctrl = ArunCtrl::create(&self.config.appid()).await?;
        let (sx, mut rx) = mpsc::channel::<RunnerRequest>(3);

        loop {
            tokio::select! {
                Some(Ok(log)) = output.next() => {
                    jinfo!("{}", log.to_string().trim());
                },

                cmd = ctrl.wait_cmd() => {
                    match cmd {
                        Ok(cmd) => {
                            jinfo!(cmd=cmd.to_string());
                            match cmd {
                                ArunCtrlCmd::Quit => break,
                                ArunCtrlCmd::Start => {
                                    self.target_state =RunnerState::Running;
                                    sx
                                        .send(RunnerRequest::UpdateState)
                                        .await.into_report()
                                        .change_context(ArunError::IOError)?
                                },
                                ArunCtrlCmd::Stop => {
                                    self.target_state = RunnerState::Exited;
                                    sx
                                        .send(RunnerRequest::UpdateState)
                                        .await.into_report()
                                        .change_context(ArunError::IOError)?
                                },
                                ArunCtrlCmd::Remove => {
                                    self.target_state = RunnerState::NonExist;
                                    sx
                                        .send(RunnerRequest::UpdateState)
                                        .await.into_report()
                                        .change_context(ArunError::IOError)?
                                }
                                _=> {},
                            }
                        },
                        Err(e) => {
                            jwarn!("Failed to process command:\n {:?}", e);
                        }
                    }

                }

                request = rx.recv() => {
                    if let Some(req) = request {
                        match req {
                            RunnerRequest::UpdateState => {
                                self.state_transition(self.target_state).await?;
                            }
                        }
                    }
                }

                _ = itimer.wait_timeup() => {
                    self.update_state().await?;
                    if self.state != self.target_state {
                        sx
                            .send(RunnerRequest::UpdateState)
                            .await.into_report()
                            .change_context(ArunError::IOError)?
                    }

                    if self.state != old_state {
                        jinfo!(NewContainerState = self.state.to_string(),OldContainerState = old_state.to_string());
                        old_state = self.state;
                    } else {
                        jinfo!(state=self.state.to_string());
                    }

                }


            }
        }

        Ok(())
    }
}

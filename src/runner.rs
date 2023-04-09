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

use super::{
    arun_config::{ArunConfig, NetworkType},
    arun_error::ArunError,
};

pub struct Runner;

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
    pub async fn run(json: &str) -> Result<(), ArunError> {
        let app = Docker::connect_with_socket_defaults()
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let arun_config = ArunConfig::parse(json)?;

        jdebug!("Arun Config:\n{:?}", arun_config);

        let option = container::CreateContainerOptions {
            name: arun_config.appid(),
            platform: None,
        };

        let config = container::Config {
            image: Some(arun_config.image()),
            cmd: Some(arun_config.cmd()),
            env: Some(arun_config.environment()),
            host_config: Some(Runner::host_config(&arun_config)),
            network_disabled: Some(arun_config.network() == NetworkType::none),
            ..Default::default()
        };

        app.create_container(Some(option), config)
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let container_name = arun_config.appid();

        app.start_container::<String>(&container_name, None)
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let options = container::WaitContainerOptions {
            condition: "not-running",
        };

        let mut wait_stopped = app.wait_container(&container_name, Some(options));

        let options = container::WaitContainerOptions {
            condition: "removed",
        };

        let mut wait_removed = app.wait_container(&container_name, Some(options));

        let options = container::AttachContainerOptions::<String> {
            stdout: Some(true),
            stream: Some(true),
            stderr: Some(true),
            logs: Some(true),
            ..Default::default()
        };

        let container::AttachContainerResults { mut output, input } = app
            .attach_container(&container_name, Some(options))
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        let _ = input;

        loop {
            tokio::select! {
                Some(ret) = wait_stopped.next() => {
                    match ret {
                        Ok(_) => jinfo!(app=container_name, state="stopped"),
                        Err(e) => jerror!(app=container_name, file=file!(), line=line!(), state="stopped", error=format!("{:?}", e)),
                    }

                    break;
                },

                Some(Ok(log)) = output.next() => {
                    jinfo!("{}", log.to_string().trim());
                },

                Some(ret) = wait_removed.next() => {
                    match ret {
                        Ok(_) => jinfo!(app=container_name, state="removed"),
                        Err(e) => jerror!(app=container_name, file=file!(), line=line!(), state="removed", error=format!("{:?}", e)),
                    }
                    break;
                },

            }
        }

        jdebug!("Remove container {}", container_name);

        let options = Some(container::RemoveContainerOptions {
            force: true,
            ..Default::default()
        });

        app.remove_container(&container_name, options)
            .await
            .into_report()
            .change_context(ArunError::DockerErr)?;

        Ok(())
    }
}

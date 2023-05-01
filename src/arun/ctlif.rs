#[allow(unused)]
use {
    arunlib::{arun_error::ArunError, utils::IntervalTimer},
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
    once_cell::sync::Lazy,
    serde::{Deserialize, Serialize},
    serde_json,
    std::{collections::HashMap, fmt::Display, str},
    tokio::time,
};

#[cfg(feature = "ctlif-ipcon")]
#[allow(unused)]
use ipcon_sys::{
    ipcon::{IPF_RCV_IF, IPF_SND_IF},
    ipcon_async::AsyncIpcon,
    ipcon_msg::IpconMsg,
};

#[derive(Serialize, Deserialize, Debug)]
pub enum ArunCtrlCmd {
    Start,
    Stop,
    Remove,
    Quit,
    Invalid,
}

impl Display for ArunCtrlCmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            ArunCtrlCmd::Start => "ArunCtrCmd::Start",
            ArunCtrlCmd::Stop => "ArunCtrCmd::Stop",
            ArunCtrlCmd::Remove => "ArunCtrCmd::Remove",
            ArunCtrlCmd::Quit => "ArunCtrCmd::Quit",
            ArunCtrlCmd::Invalid => "ArunCtrlCmd::Invalid",
        };

        write!(f, "{}", msg)
    }
}

pub struct ArunCtrl {
    #[cfg(feature = "ctlif-ipcon")]
    ih: AsyncIpcon,
}

impl ArunCtrl {
    pub async fn create(name: &str) -> Result<Self, ArunError> {
        #[allow(unused)]
        let name = format!("arun.{}", name);

        Ok(Self {
            #[cfg(feature = "ctlif-ipcon")]
            ih: AsyncIpcon::new(Some(&name), Some(IPF_RCV_IF))
                .change_context(ArunError::IpconError)?,
        })
    }

    pub async fn wait_cmd(&mut self) -> Result<ArunCtrlCmd, ArunError> {
        let cmd: ArunCtrlCmd;

        #[cfg(not(feature = "ctlif-ipcon"))]
        {
            let mut itimer = IntervalTimer::new(time::Duration::from_secs(u64::MAX));
            itimer.wait_timeup().await;
            cmd = ArunCtrlCmd::Invalid;
        }

        #[cfg(feature = "ctlif-ipcon")]
        {
            match self
                .ih
                .receive_msg()
                .await
                .change_context(ArunError::IpconError)?
            {
                IpconMsg::IpconMsgUser(m) => {
                    let body = str::from_utf8(&m.buf)
                        .into_report()
                        .change_context(ArunError::InvalidValue)?
                        .trim()
                        .trim_end_matches('\0');

                    cmd = serde_json::from_str(body)
                        .into_report()
                        .change_context(ArunError::InvalidValue)
                        .attach_printable(format!("Failed to parse json command {}", body))?;

                    jdebug!(from = m.peer, cmd = cmd.to_string());
                }
                _ => return Err(ArunError::InvalidValue).into_report(),
            }
        }

        Ok(cmd)
    }
}

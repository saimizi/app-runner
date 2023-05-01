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
    std::{
        collections::HashMap,
        fmt::Display,
        str,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    },
    tokio::{
        sync::mpsc::{UnboundedReceiver, UnboundedSender},
        task::{spawn, JoinHandle},
        time::{sleep, timeout, Duration},
    },
};

#[cfg(feature = "ctlif-ipcon")]
#[allow(unused)]
use ipcon_sys::{
    ipcon::{IPF_RCV_IF, IPF_SND_IF},
    ipcon_async::AsyncIpcon,
    ipcon_msg::IpconMsg,
};
use tokio_stream::wrappers::UnboundedReceiverStream;

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
    should_quit: Arc<AtomicBool>,
    stream: UnboundedReceiverStream<ArunCtrlCmd>,
    handler: Option<JoinHandle<Result<(), ArunError>>>,
}

impl ArunCtrl {
    pub async fn create(name: &str) -> Result<Self, ArunError> {
        #[cfg(feature = "ctlif-ipcon")]
        {
            let name = format!("arun.{}", name);

            let (sx, rx) = tokio::sync::mpsc::unbounded_channel::<ArunCtrlCmd>();
            let should_quit = Arc::new(AtomicBool::new(false));

            let ih = AsyncIpcon::new(Some(&name), Some(IPF_RCV_IF))
                .change_context(ArunError::IpconError)?;

            let should_quit_in = should_quit.clone();
            let handler: JoinHandle<Result<(), ArunError>> = spawn(async move {
                while !should_quit_in.load(Ordering::Relaxed) {
                    // This timeout give the chance to check should_quit.
                    match timeout(Duration::from_secs(3), ih.receive_msg()).await {
                        Ok(ret) => match ret.change_context(ArunError::IpconError)? {
                            IpconMsg::IpconMsgUser(m) => {
                                let body = str::from_utf8(&m.buf)
                                    .into_report()
                                    .change_context(ArunError::InvalidValue)?
                                    .trim()
                                    .trim_end_matches('\0');

                                let cmd: ArunCtrlCmd = serde_json::from_str(body)
                                    .into_report()
                                    .change_context(ArunError::InvalidValue)
                                    .attach_printable(format!(
                                        "Failed to parse json command {}",
                                        body
                                    ))?;

                                jdebug!(from = m.peer, cmd = cmd.to_string());

                                sx.send(cmd)
                                    .into_report()
                                    .change_context(ArunError::IOError)?;
                            }
                            _ => return Err(ArunError::InvalidValue).into_report(),
                        },
                        Err(_) => {
                            jdebug!("Time out");
                        }
                    }
                }

                jdebug!("ArunCtrl thread quit");
                Ok(())
            });

            Ok(Self {
                should_quit,
                stream: UnboundedReceiverStream::from(rx),
                handler: Some(handler),
            })
        }

        #[cfg(not(feature = "ctlif-ipcon"))]
        {
            let (sx, rx) = tokio::sync::mpsc::unbounded_channel::<ArunCtrlCmd>();
            let should_quit = Arc::new(AtomicBool::new(false));
            let should_quit_in = should_quit.clone();
            let handler = spawn(async move {
                while !should_quit_in.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_secs(u64::MAX)).await;
                }
            });
            Ok(Self { should_quit, rx })
        }
    }

    pub async fn wait_cmd(&mut self) -> Result<ArunCtrlCmd, ArunError> {
        if let Some(cmd) = self.stream.next().await {
            Ok(cmd)
        } else {
            Err(ArunError::IOError).into_report()
        }
    }

    pub async fn exit(mut self) {
        self.should_quit.store(true, Ordering::Relaxed);

        if let Some(handler) = self.handler.take() {
            if let Err(e) = handler.await {
                jwarn!("ArunCtrl exit with error: {:?}", e);
            }
        }
    }
}

pub mod arun;

#[allow(unused)]
use {
    arun::{
        arun_config::{AppType, ArunConfig},
        runner::Runner,
    },
    arunlib::arun_error::ArunError,
    clap::Parser,
    error_stack::{IntoReport, Report, Result, ResultExt},
    jlogger_tracing::{
        jdebug, jerror, jinfo, jtrace, jwarn, JloggerBuilder, LevelFilter, LogTimeFormat,
    },
    std::fs,
};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(short = 'c', long = "config")]
    config: String,

    #[clap(short = 'l',long="log-file", default_value_t=String::from("/tmp/arun.log"))]
    log: String,

    #[clap(short = 'm', long = "monitor-interval")]
    monitor_interval: Option<u32>,

    #[clap(short, long, parse(from_occurrences))]
    verbose: usize,
}

#[tokio::main]
async fn main() -> Result<(), ArunError> {
    let cli = Cli::parse();

    let max_level = match cli.verbose {
        1 => LevelFilter::DEBUG,
        2 => LevelFilter::TRACE,
        _ => LevelFilter::INFO,
    };

    JloggerBuilder::new().max_level(max_level).build();

    let json = fs::read_to_string(cli.config)
        .into_report()
        .change_context(ArunError::InvalidValue)?;

    jdebug!("Config:\n{}", json);

    let mut runner = Runner::new(json.as_str(), cli.monitor_interval).await?;

    runner.run().await
}

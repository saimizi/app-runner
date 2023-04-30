//cspell:word canonicalize rnetmon aarch rustc

#[allow(unused)]
use {
    jlogger_tracing::{jdebug, jerror, jinfo, jwarn, JloggerBuilder, LevelFilter, LogTimeFormat},
    std::{
        env,
        fs::{canonicalize, create_dir_all, remove_file},
        os::unix::fs::symlink,
        path::Path,
    },
};

fn main() {
    JloggerBuilder::new()
        .max_level(LevelFilter::DEBUG)
        .log_file(Some(("/tmp/arun.log", false)))
        .log_console(false)
        .log_time(LogTimeFormat::TimeNone)
        .build();

    let target = env::var("TARGET").unwrap();
    jinfo!(target = target);
    if target == "aarch64-unknown-linux-gnu" {
        if let Ok(arm64_lib) = canonicalize("arm64") {
            jinfo!("Use arm64/");
            println!("cargo:rustc-link-arg=-lnl-3");
            println!("cargo:rustc-link-arg=-lnl-genl-3");

            let cargo_search_path = format!(
                "cargo:rustc-link-search={}",
                arm64_lib.as_path().to_str().unwrap()
            );
            println!("{}", cargo_search_path);
        }
    }
}

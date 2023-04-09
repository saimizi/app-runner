//cspell:word fdebug ferror finfo timeup

#[allow(unused_imports)]
use {
    jlogger_tracing::{
        jdebug, jerror, jinfo, jtrace, jwarn, JloggerBuilder, LevelFilter, LogTimeFormat,
    },
    tokio::time::{sleep, Duration, Instant},
};

pub struct IntervalTimer {
    start: Instant,
    interval: Duration,
}

impl IntervalTimer {
    pub fn new(interval: Duration) -> IntervalTimer {
        IntervalTimer {
            start: Instant::now(),
            interval,
        }
    }

    pub fn update_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    pub async fn wait_timeup_interval(&mut self, interval: Option<Duration>) {
        if let Some(i) = interval {
            self.start = Instant::now();
            self.interval = i;
        }

        self.wait_timeup().await;
    }

    pub async fn wait_timeup(&mut self) {
        let d = self.start.elapsed();
        let to_sleep: Duration;

        if d >= self.interval {
            /* reload timer */
            self.start = Instant::now();
            to_sleep = self.interval;
        } else {
            to_sleep = self.interval - d;
        }

        sleep(to_sleep).await;
    }
}

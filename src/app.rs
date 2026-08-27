use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::metrics::{MetricsSampler, SystemSnapshot};

pub struct App {
    sampler: MetricsSampler,
    tick_rate: Duration,
    last_tick: Instant,
    snapshot: SystemSnapshot,
    last_refresh_error: Option<String>,
}

impl App {
    pub fn new(tick_rate: Duration) -> Result<Self> {
        let mut sampler = MetricsSampler::new()?;
        let baseline = sampler.sample()?;
        thread::sleep(Duration::from_millis(250));
        let last_tick = Instant::now();
        let (snapshot, last_refresh_error) = match sampler.sample() {
            Ok(snapshot) => (snapshot, None),
            Err(error) => (baseline, Some(format!("refresh failed: {error:#}"))),
        };

        Ok(Self {
            sampler,
            tick_rate,
            last_tick,
            snapshot,
            last_refresh_error,
        })
    }

    pub fn snapshot(&self) -> &SystemSnapshot {
        &self.snapshot
    }

    pub fn last_refresh_error(&self) -> Option<&str> {
        self.last_refresh_error.as_deref()
    }

    pub fn poll_timeout(&self) -> Duration {
        self.tick_rate
            .checked_sub(self.last_tick.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn refresh_now(&mut self) {
        let refresh_started = Instant::now();
        match self.sampler.sample() {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.last_refresh_error = None;
            }
            Err(error) => {
                self.last_refresh_error = Some(format!("refresh failed: {error:#}"));
            }
        }
        self.last_tick = refresh_started;
    }

    pub fn tick_if_needed(&mut self) {
        if self.last_tick.elapsed() >= self.tick_rate {
            self.refresh_now();
        }
    }
}

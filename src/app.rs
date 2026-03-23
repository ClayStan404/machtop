use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::metrics::{MetricsSampler, SystemSnapshot};

pub struct App {
    sampler: MetricsSampler,
    tick_rate: Duration,
    last_tick: Instant,
    snapshot: SystemSnapshot,
}

impl App {
    pub fn new(tick_rate: Duration) -> Result<Self> {
        let mut sampler = MetricsSampler::new()?;
        thread::sleep(Duration::from_millis(250));
        let snapshot = sampler.sample()?;

        Ok(Self {
            sampler,
            tick_rate,
            last_tick: Instant::now(),
            snapshot,
        })
    }

    pub fn snapshot(&self) -> &SystemSnapshot {
        &self.snapshot
    }

    pub fn poll_timeout(&self) -> Duration {
        self.tick_rate
            .checked_sub(self.last_tick.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn refresh_now(&mut self) -> Result<()> {
        self.snapshot = self.sampler.sample()?;
        self.last_tick = Instant::now();
        Ok(())
    }

    pub fn tick_if_needed(&mut self) -> Result<()> {
        if self.last_tick.elapsed() >= self.tick_rate {
            self.refresh_now()?;
        }
        Ok(())
    }
}

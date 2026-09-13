//! One idle session, independent of window timers and wall-clock changes.
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub threshold: Duration,
    pub warning: Duration,
    pub action: String,
    pub enhanced: bool,
}

#[derive(Default)]
pub struct Clock {
    settings: Option<Settings>,
    started: Option<Instant>,
    last_input: Option<u32>,
    periodic_count: u32,
    periodic_gap: u32,
    logical_idle: bool,
    warning: bool,
    deadline: Option<Instant>,
}

#[derive(Default)]
pub struct Update {
    pub cancel: bool,
    pub show: bool,
}

impl Clock {
    pub fn restart(&mut self, now: Instant) {
        self.started = Some(now);
        self.warning = false;
        self.deadline = None;
        self.logical_idle = false;
    }

    pub fn sample(
        &mut self,
        now: Instant,
        input: Option<(u32, Duration)>,
        settings: Settings,
    ) -> Update {
        let was_warning = self.warning;
        if self.settings.as_ref() != Some(&settings) {
            self.restart(now);
            self.periodic_count = 0;
            self.settings = Some(settings.clone());
        }
        let Some((tick, raw_idle)) = input else {
            // An unavailable input source must never advance an action.
            self.restart(now);
            return Update {
                cancel: was_warning,
                show: false,
            };
        };
        if let Some(previous) = self.last_input.filter(|previous| *previous != tick) {
            let gap = tick.wrapping_sub(previous);
            let periodic = settings.enhanced && (20_000..=120_000).contains(&gap);
            if periodic {
                if self.periodic_count == 0 || gap.abs_diff(self.periodic_gap) <= 5_000 {
                    self.periodic_gap = if self.periodic_count == 0 {
                        gap
                    } else {
                        (self.periodic_gap + gap) / 2
                    };
                    self.periodic_count = self.periodic_count.saturating_add(1);
                } else {
                    self.periodic_count = 1;
                    self.periodic_gap = gap;
                }
            } else {
                self.periodic_count = 0;
            }
            if periodic && self.periodic_count >= 3 {
                self.logical_idle = true;
            } else {
                self.restart(now);
            }
        }
        self.last_input = Some(tick);
        if !settings.enabled {
            self.restart(now);
            return Update {
                cancel: was_warning,
                show: false,
            };
        }
        let elapsed = now.saturating_duration_since(*self.started.get_or_insert(now));
        let idle = if self.logical_idle {
            elapsed
        } else {
            elapsed.min(raw_idle)
        };
        let mut show = false;
        if !self.warning && idle >= settings.threshold.saturating_sub(settings.warning) {
            self.warning = true;
            self.deadline = Some(now + settings.threshold.saturating_sub(idle));
            show = true;
        }
        Update {
            cancel: was_warning && (!self.warning || show),
            show,
        }
    }

    pub fn countdown(&self, now: Instant) -> Option<(u32, String)> {
        if !self.warning {
            return None;
        }
        let left = self.deadline?.saturating_duration_since(now);
        let seconds = left.as_secs() + u64::from(left.subsec_nanos() != 0);
        Some((seconds as u32, self.settings.as_ref()?.action.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> Settings {
        Settings {
            enabled: true,
            threshold: Duration::from_secs(60),
            warning: Duration::from_secs(10),
            action: "lock".into(),
            enhanced: false,
        }
    }
    #[test]
    fn startup_reenable_cancel_and_changed_action_begin_fresh_sessions() {
        let start = Instant::now();
        let mut clock = Clock::default();
        let input = Some((1, Duration::from_secs(3600)));
        assert!(!clock.sample(start, input, settings()).show);
        assert!(
            clock
                .sample(start + Duration::from_secs(50), input, settings())
                .show
        );
        assert_eq!(
            clock.countdown(start + Duration::from_secs(55)).unwrap().0,
            5
        );
        let mut changed = settings();
        changed.action = "sleep".into();
        assert!(
            clock
                .sample(start + Duration::from_secs(56), input, changed)
                .cancel
        );
        assert!(clock.countdown(start + Duration::from_secs(60)).is_none());
        clock.restart(start + Duration::from_secs(100));
        assert!(
            !clock
                .sample(start + Duration::from_secs(101), input, settings())
                .show
        );
        let mut disabled = settings();
        disabled.enabled = false;
        clock.sample(start + Duration::from_secs(200), input, disabled);
        assert!(
            !clock
                .sample(start + Duration::from_secs(300), input, settings())
                .show
        );
    }
    #[test]
    fn zero_and_long_warning_do_not_disable_actions() {
        let start = Instant::now();
        for warning in [0, 120] {
            let mut clock = Clock::default();
            let mut config = settings();
            config.warning = Duration::from_secs(warning);
            let input = Some((1, Duration::from_secs(3600)));
            assert_eq!(
                clock.sample(start, input, config.clone()).show,
                warning == 120
            );
            clock.sample(start + Duration::from_secs(60), input, config);
            assert_eq!(
                clock.countdown(start + Duration::from_secs(60)).unwrap().0,
                0
            );
        }
    }
    #[test]
    fn periodic_resets_accumulate_but_real_activity_and_read_failures_cancel() {
        let start = Instant::now();
        let mut clock = Clock::default();
        let mut config = settings();
        config.enhanced = true;
        for i in 0..=4 {
            clock.sample(
                start + Duration::from_secs(i * 30),
                Some(((i * 30_000) as u32, Duration::ZERO)),
                config.clone(),
            );
        }
        assert_eq!(
            clock.countdown(start + Duration::from_secs(120)).unwrap().0,
            0
        );
        assert!(
            clock
                .sample(
                    start + Duration::from_secs(121),
                    Some((121_000, Duration::ZERO)),
                    config.clone()
                )
                .cancel
        );
        assert!(clock.countdown(start + Duration::from_secs(121)).is_none());
        clock.sample(
            start + Duration::from_secs(180),
            Some((121_000, Duration::from_secs(59))),
            config.clone(),
        );
        assert!(
            clock
                .sample(start + Duration::from_secs(181), None, config)
                .cancel
        );
    }
}

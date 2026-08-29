//! macOS-style scroll physics shared by the flat grid and the pivot.
//!
//! Three behaviors, all gated on the `animations` flag (when it is off every
//! path collapses to the pre-existing instant, hard-clamped scrolling):
//!
//! 1. **Rubber-band overscroll** — a trackpad gesture that runs past the
//!    scrollable range keeps pulling the whole painted surface with
//!    asymptotic resistance (Apple's classic `limit·d / (limit + d)` curve),
//!    exactly like an `NSScrollView`. The *raw* pull accumulates 1:1 so the
//!    gesture stays reversible; only the displayed offset is compressed.
//! 2. **Bounce-back** — when the gesture releases (`TouchPhase::Ended`), a
//!    critically damped spring returns the pull to zero. Momentum deltas —
//!    which macOS delivers as `Moved` events *after* `Ended`, since gpui maps
//!    only `NSEvent.phase()` and not `momentumPhase()` — convert into spring
//!    velocity impulses at the edge, so a momentum fling into an edge dips
//!    into overscroll and springs back instead of dead-stopping.
//! 3. **Smooth wheel scrolling** — discrete mouse-wheel ticks (`Lines`
//!    deltas) animate toward their accumulated target with an exponential
//!    ease-out instead of teleporting, matching the feel of macOS smooth
//!    scrolling. Wheels do not rubber-band (macOS scroll views bounce for
//!    gesture devices, not click wheels), so the target hard-clamps.
//!
//! The struct owns no scroll offset — callers pass the current offset and
//! range in and store the returned offset — so the same code drives both
//! `GridState` and `PivotState`, and every rule here is testable without a
//! window. Axes only overscroll when they are actually scrollable
//! (`max > 0`), matching `NSScrollView`'s default bounce behavior for an
//! embedded pane.

use std::time::{Duration, Instant};

use gpui::TouchPhase;

/// Spring stiffness (1/s²) for the bounce-back. ~230 settles a full pull in
/// roughly a third of a second, which is the macOS feel.
const STIFFNESS: f32 = 230.0;
/// Critical damping for [`STIFFNESS`]: `2·√k`. No visible oscillation — the
/// surface returns and stops, it never wobbles.
const DAMPING: f32 = 30.33;
/// Pull (px) and velocity (px/s) below which the spring snaps to rest.
const REST_PULL: f32 = 0.5;
const REST_VELOCITY: f32 = 20.0;
/// Cap on spring velocity from momentum impulses, so a violent fling cannot
/// launch the surface across the window.
const MAX_IMPULSE_VELOCITY: f32 = 3_000.0;
/// A gesture with no wheel events for this long is treated as released even
/// if the terminal `Ended` event was lost (e.g. the window lost focus
/// mid-gesture). Without this a stale pull would freeze on screen.
const GESTURE_TIMEOUT: Duration = Duration::from_millis(250);
/// Time constant for smooth wheel scrolling; ~90 ms reaches 63% of the
/// remaining distance, fast enough to feel direct.
const SMOOTH_TAU: f32 = 0.09;
/// Distance (px) at which smooth scrolling snaps to its target.
const SMOOTH_EPSILON: f32 = 0.5;
/// Fraction of the viewport the rubber-band asymptotically approaches.
const RUBBER_LIMIT_FRACTION: f32 = 0.45;
/// Floor for the rubber-band limit so tiny panes still visibly pull.
const RUBBER_LIMIT_MIN: f32 = 60.0;

/// Scroll physics for one scrollable surface. See the module docs.
#[derive(Debug, Clone)]
pub(crate) struct ScrollPhysics {
    /// Raw signed overscroll per axis, pre-resistance. Negative = pulled
    /// beyond the start (top/left) edge, positive = beyond the end edge.
    pull: (f32, f32),
    /// Spring velocity per axis, px/s. Nonzero while bouncing.
    velocity: (f32, f32),
    /// Whether a trackpad gesture (fingers down) is in flight.
    gesture_active: bool,
    /// Accumulated smooth-scroll target for discrete wheel input.
    smooth_target: Option<(f32, f32)>,
    /// When the last wheel event arrived (gesture-timeout safety and
    /// momentum-impulse timing).
    last_event: Option<Instant>,
    /// When [`Self::step`] last ran, for frame-delta integration.
    last_step: Option<Instant>,
}

impl Default for ScrollPhysics {
    fn default() -> Self {
        Self {
            pull: (0.0, 0.0),
            velocity: (0.0, 0.0),
            gesture_active: false,
            smooth_target: None,
            last_event: None,
            last_step: None,
        }
    }
}

/// Apple's rubber-band curve: monotonic, `rubber(0) = 0`, asymptotic to
/// `±limit`. `d/2` displayed at `d = limit` of raw pull.
fn rubber(pull: f32, limit: f32) -> f32 {
    limit * pull / (limit + pull.abs())
}

impl ScrollPhysics {
    /// Feed one wheel event through the physics. Returns the new scroll
    /// offset to store. `precise` is true for pixel-precise (trackpad /
    /// Magic Mouse) deltas, false for discrete wheel lines; `delta` is
    /// already converted to pixels with the same sign convention as the
    /// existing handlers (`new = scroll - delta`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn on_wheel(
        &mut self,
        delta: (f32, f32),
        precise: bool,
        phase: TouchPhase,
        scroll: (f32, f32),
        max: (f32, f32),
        animations: bool,
        now: Instant,
    ) -> (f32, f32) {
        if !animations {
            // Animations off is the accessibility opt-out: identical to the
            // pre-physics behavior, and any in-flight state is discarded.
            self.reset();
            return (
                (scroll.0 - delta.0).clamp(0.0, max.0),
                (scroll.1 - delta.1).clamp(0.0, max.1),
            );
        }

        let dt_event = self
            .last_event
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or(Duration::from_millis(16))
            .as_secs_f32()
            .clamp(0.004, 0.032);
        self.last_event = Some(now);

        if !precise {
            // Discrete wheel: accumulate a hard-clamped target and let
            // `step` glide toward it. A wheel tick also takes over from any
            // in-flight trackpad bounce.
            let base = self.smooth_target.unwrap_or(scroll);
            self.smooth_target = Some((
                (base.0 - delta.0).clamp(0.0, max.0),
                (base.1 - delta.1).clamp(0.0, max.1),
            ));
            return scroll;
        }

        // Trackpad path. A new touch cancels wheel smoothing and catches any
        // bounce in flight (the finger now owns the pull).
        match phase {
            TouchPhase::Started => {
                self.gesture_active = true;
                self.velocity = (0.0, 0.0);
                self.smooth_target = None;
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.gesture_active = false;
            }
            _ => {}
        }

        let apply = |axis_scroll: f32,
                     axis_delta: f32,
                     axis_max: f32,
                     axis_pull: &mut f32,
                     axis_vel: &mut f32,
                     gesture: bool| {
            if axis_max <= 0.0 {
                // Not scrollable on this axis: no motion, no bounce.
                *axis_pull = 0.0;
                return axis_scroll;
            }
            if gesture {
                // Position control: the raw pull tracks the fingers 1:1
                // through the clamp, so reversing direction unwinds it
                // before the offset moves again.
                let virtual_offset = axis_scroll - axis_delta + *axis_pull;
                let clamped = virtual_offset.clamp(0.0, axis_max);
                *axis_pull = virtual_offset - clamped;
                clamped
            } else {
                // Momentum (or a phase-less precise event): in-range deltas
                // scroll normally; the part that would cross an edge becomes
                // a velocity impulse for the spring, which then owns the
                // bounce. Sign: excess < 0 pushes beyond the start edge, and
                // the spring integrates the matching negative pull.
                let unclamped = axis_scroll - axis_delta;
                let clamped = unclamped.clamp(0.0, axis_max);
                let excess = unclamped - clamped;
                if excess != 0.0 {
                    *axis_vel = (*axis_vel + excess / dt_event)
                        .clamp(-MAX_IMPULSE_VELOCITY, MAX_IMPULSE_VELOCITY);
                }
                clamped
            }
        };

        let gesture = self.gesture_active;
        let nx = apply(
            scroll.0,
            delta.0,
            max.0,
            &mut self.pull.0,
            &mut self.velocity.0,
            gesture,
        );
        let ny = apply(
            scroll.1,
            delta.1,
            max.1,
            &mut self.pull.1,
            &mut self.velocity.1,
            gesture,
        );
        (nx, ny)
    }

    /// Advance the physics by one frame. Returns the new scroll offset and
    /// whether the physics still needs stepping (drive the caller's repaint
    /// loop until this goes false).
    pub(crate) fn step(
        &mut self,
        scroll: (f32, f32),
        max: (f32, f32),
        now: Instant,
    ) -> ((f32, f32), bool) {
        let dt = self
            .last_step
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or(Duration::from_millis(16))
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_step = Some(now);

        // Lost-`Ended` safety: a silent gesture is a released gesture.
        if self.gesture_active
            && self
                .last_event
                .is_some_and(|t| now.saturating_duration_since(t) > GESTURE_TIMEOUT)
        {
            self.gesture_active = false;
        }

        let mut scroll = scroll;

        // Smooth wheel glide.
        if let Some(target) = self.smooth_target {
            let target = (target.0.clamp(0.0, max.0), target.1.clamp(0.0, max.1));
            let alpha = 1.0 - (-dt / SMOOTH_TAU).exp();
            scroll.0 += (target.0 - scroll.0) * alpha;
            scroll.1 += (target.1 - scroll.1) * alpha;
            if (target.0 - scroll.0).abs() < SMOOTH_EPSILON
                && (target.1 - scroll.1).abs() < SMOOTH_EPSILON
            {
                scroll = target;
                self.smooth_target = None;
            }
        }

        // Spring the pull home once the fingers are off the pad.
        if !self.gesture_active {
            for (pull, vel) in [
                (&mut self.pull.0, &mut self.velocity.0),
                (&mut self.pull.1, &mut self.velocity.1),
            ] {
                if *pull != 0.0 || *vel != 0.0 {
                    // Semi-implicit Euler keeps the critically damped spring
                    // stable at UI frame rates.
                    *vel += (-STIFFNESS * *pull - DAMPING * *vel) * dt;
                    *pull += *vel * dt;
                    if pull.abs() < REST_PULL && vel.abs() < REST_VELOCITY {
                        *pull = 0.0;
                        *vel = 0.0;
                    }
                }
            }
        }

        (scroll, self.needs_step())
    }

    /// Whether [`Self::step`] still has work: a bounce in flight, a pull
    /// waiting on a (possibly lost) release, or a wheel glide mid-way.
    pub(crate) fn needs_step(&self) -> bool {
        self.pull != (0.0, 0.0) || self.velocity != (0.0, 0.0) || self.smooth_target.is_some()
    }

    /// Displayed translation for the painted surface, in px to *add* to the
    /// paint origin: pulled beyond the top edge shifts content down, beyond
    /// the left edge shifts it right. Compresses the raw pull through the
    /// rubber curve against the viewport size.
    pub(crate) fn display_shift(&self, viewport: (f32, f32)) -> (f32, f32) {
        let limit = |dim: f32| (dim * RUBBER_LIMIT_FRACTION).max(RUBBER_LIMIT_MIN);
        (
            -rubber(self.pull.0, limit(viewport.0)),
            -rubber(self.pull.1, limit(viewport.1)),
        )
    }

    /// Discard all in-flight physics (used when animations are disabled).
    pub(crate) fn reset(&mut self) {
        *self = Self {
            last_event: self.last_event,
            last_step: self.last_step,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    fn ms(base: Instant, m: u64) -> Instant {
        base + Duration::from_millis(m)
    }

    /// Drive `step` until rest or `frames` elapse; returns final scroll.
    fn settle(p: &mut ScrollPhysics, mut scroll: (f32, f32), max: (f32, f32)) -> (f32, f32) {
        let base = t0();
        for i in 1..=600u64 {
            let (s, active) = p.step(scroll, max, ms(base, i * 16));
            scroll = s;
            if !active {
                break;
            }
        }
        scroll
    }

    #[test]
    fn animations_off_is_plain_clamped_scrolling() {
        let mut p = ScrollPhysics::default();
        let s = p.on_wheel(
            (0.0, -50.0),
            true,
            TouchPhase::Moved,
            (0.0, 100.0),
            (0.0, 500.0),
            false,
            t0(),
        );
        assert_eq!(s, (0.0, 150.0));
        assert!(!p.needs_step());

        // Past the edge: hard clamp, no pull.
        let s = p.on_wheel(
            (0.0, 500.0),
            true,
            TouchPhase::Moved,
            s,
            (0.0, 500.0),
            false,
            t0(),
        );
        assert_eq!(s, (0.0, 0.0));
        assert_eq!(p.display_shift((800.0, 600.0)), (0.0, 0.0));
    }

    #[test]
    fn gesture_past_top_accumulates_pull_and_shifts_content_down() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        // Two pulls of 60 px down at the top edge.
        s = p.on_wheel(
            (0.0, 60.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        s = p.on_wheel(
            (0.0, 60.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 16),
        );
        assert_eq!(s, (0.0, 0.0));
        let (sx, sy) = p.display_shift((800.0, 600.0));
        assert_eq!(sx, 0.0);
        assert!(sy > 0.0, "pull past the top must shift content down");
        // Displayed shift is compressed below the raw 120 px pull.
        assert!(sy < 120.0);
        assert!(p.needs_step());
    }

    #[test]
    fn reversing_the_gesture_unwinds_pull_before_scrolling() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        s = p.on_wheel(
            (0.0, 80.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        assert_eq!(s, (0.0, 0.0));
        // Scroll back 30 px: consumed entirely by the pull, offset unmoved.
        s = p.on_wheel(
            (0.0, -30.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 16),
        );
        assert_eq!(s, (0.0, 0.0));
        assert!(p.display_shift((800.0, 600.0)).1 > 0.0);
        // Another 80 px: unwinds the remaining 50 px of pull, then scrolls.
        s = p.on_wheel(
            (0.0, -80.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 24),
        );
        assert_eq!(s, (0.0, 30.0));
        assert_eq!(p.display_shift((800.0, 600.0)).1, 0.0);
    }

    #[test]
    fn release_springs_back_to_rest() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        s = p.on_wheel(
            (0.0, 100.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Ended,
            s,
            max,
            true,
            ms(base, 16),
        );
        assert!(p.display_shift((800.0, 600.0)).1 > 0.0);
        let s = settle(&mut p, s, max);
        assert_eq!(s, (0.0, 0.0));
        assert!(!p.needs_step());
        assert_eq!(p.display_shift((800.0, 600.0)), (0.0, 0.0));
    }

    #[test]
    fn momentum_into_edge_bounces_and_returns() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        // Gesture scrolls to near the bottom and releases.
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 480.0),
            max,
            true,
            base,
        );
        s = p.on_wheel(
            (0.0, -15.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Ended,
            s,
            max,
            true,
            ms(base, 16),
        );
        assert_eq!(s, (0.0, 495.0));
        // Momentum (Moved after Ended) carries it past the bottom edge.
        s = p.on_wheel(
            (0.0, -40.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 24),
        );
        assert_eq!(s, (0.0, 500.0), "offset itself stays clamped");
        assert!(p.needs_step(), "excess became a spring impulse");
        // The bounce dips into overscroll on the way…
        let (_, active) = p.step(s, max, ms(base, 40));
        assert!(active);
        assert!(
            p.display_shift((800.0, 600.0)).1 < 0.0,
            "bounce shifts content up past the bottom edge"
        );
        // …and settles back flush.
        let s = settle(&mut p, s, max);
        assert_eq!(s, (0.0, 500.0));
        assert_eq!(p.display_shift((800.0, 600.0)), (0.0, 0.0));
    }

    #[test]
    fn wheel_lines_glide_to_a_clamped_target() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        let s0 = (0.0, 0.0);
        // One discrete tick of 120 px: offset unchanged until step.
        let s = p.on_wheel((0.0, -120.0), false, TouchPhase::Moved, s0, max, true, base);
        assert_eq!(s, s0);
        assert!(p.needs_step());
        // First frame moves part-way; settling lands exactly on target.
        let (s1, _) = p.step(s, max, ms(base, 16));
        assert!(s1.1 > 0.0 && s1.1 < 120.0);
        let s = settle(&mut p, s1, max);
        assert_eq!(s, (0.0, 120.0));
        // A tick past the bottom clamps the target — no wheel rubber-band.
        let s = p.on_wheel(
            (0.0, -1000.0),
            false,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 32),
        );
        let s = settle(&mut p, s, max);
        assert_eq!(s, (0.0, 500.0));
        assert_eq!(p.display_shift((800.0, 600.0)), (0.0, 0.0));
    }

    #[test]
    fn rapid_wheel_ticks_accumulate_into_one_target() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        let mut s = (0.0, 0.0);
        for i in 0..3 {
            s = p.on_wheel(
                (0.0, -60.0),
                false,
                TouchPhase::Moved,
                s,
                max,
                true,
                ms(base, i * 10),
            );
        }
        let s = settle(&mut p, s, max);
        assert_eq!(s, (0.0, 180.0), "three ticks land three ticks away");
    }

    #[test]
    fn non_scrollable_axis_never_pulls() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        // Horizontal content fits (max.x == 0): x never moves or pulls.
        let max = (0.0, 500.0);
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        s = p.on_wheel(
            (50.0, 30.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        assert_eq!(s.0, 0.0);
        assert_eq!(p.display_shift((800.0, 600.0)).0, 0.0);
        assert!(p.display_shift((800.0, 600.0)).1 > 0.0);
    }

    #[test]
    fn lost_ended_event_releases_after_timeout() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        s = p.on_wheel(
            (0.0, 90.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        // No Ended ever arrives. Steps within the timeout hold the pull…
        let (_, active) = p.step(s, max, ms(base, 100));
        assert!(active);
        assert!(p.display_shift((800.0, 600.0)).1 > 0.0);
        // …after the timeout the spring takes over and settles home.
        for i in 0..600u64 {
            let (ns, active) = p.step(s, max, ms(base, 300 + i * 16));
            s = ns;
            if !active {
                break;
            }
        }
        assert_eq!(p.display_shift((800.0, 600.0)), (0.0, 0.0));
        assert!(!p.needs_step());
    }

    #[test]
    fn rubber_curve_is_monotonic_and_bounded() {
        let limit = 200.0;
        let mut last = 0.0;
        for i in 1..1000 {
            let d = rubber(i as f32 * 5.0, limit);
            assert!(d > last, "monotonic");
            assert!(d < limit, "bounded by the limit");
            last = d;
        }
        assert_eq!(rubber(-80.0, limit), -rubber(80.0, limit));
        assert_eq!(rubber(0.0, limit), 0.0);
    }

    #[test]
    fn new_touch_catches_a_bounce_in_flight() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 500.0);
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        s = p.on_wheel(
            (0.0, 100.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Ended,
            s,
            max,
            true,
            ms(base, 16),
        );
        // Bounce starts…
        let (s2, _) = p.step(s, max, ms(base, 32));
        let mid_shift = p.display_shift((800.0, 600.0)).1;
        assert!(mid_shift > 0.0);
        // …fingers land again: the pull freezes where the finger caught it.
        let s3 = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            s2,
            max,
            true,
            ms(base, 40),
        );
        let (s4, active) = p.step(s3, max, ms(base, 56));
        assert!(active, "held pull still needs the release watchdog");
        let held = p.display_shift((800.0, 600.0)).1;
        assert!(held > 0.0 && held <= mid_shift, "finger froze the pull");
        let (_, _) = p.step(s4, max, ms(base, 72));
        assert_eq!(
            p.display_shift((800.0, 600.0)).1,
            held,
            "no spring while held"
        );
    }
}

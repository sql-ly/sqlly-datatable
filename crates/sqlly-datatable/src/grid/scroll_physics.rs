//! macOS-style scroll physics shared by the flat grid and the pivot.
//!
//! Three behaviors, all gated on the `animations` flag (when it is off every
//! path collapses to the pre-existing instant, hard-clamped scrolling):
//!
//! 1. **Rubber-band overscroll** — a trackpad gesture that runs past the
//!    scrollable range keeps pulling the whole painted surface with
//!    asymptotic resistance (Apple's classic `limit·d / (limit + d)` curve),
//!    exactly like an `NSScrollView`. The *raw* pull accumulates 1:1 so the
//!    gesture stays reversible; only the displayed offset is compressed, and
//!    it stays strictly under [`RUBBER_LIMIT_MAX`] (150 px) on any viewport.
//! 2. **Bounce-back** — when the gesture releases (`TouchPhase::Ended`), a
//!    critically damped spring returns the pull to zero. Momentum deltas —
//!    which macOS delivers as `Moved` events *after* `Ended`, since gpui maps
//!    only `NSEvent.phase()` and not `momentumPhase()` — are spent as
//!    exactly **one** spring impulse per stream and edge: the first delta to
//!    cross the edge kicks the spring with the content's impact velocity,
//!    and the rest of the decaying momentum tail is swallowed. (Letting
//!    every tail event re-pump the spring while it fights back is visible
//!    jitter.)
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

use std::time::Duration;
#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
#[cfg(target_family = "wasm")]
use web_time::Instant;

use gpui::TouchPhase;

/// The wall clock the scroll physics runs on. `std::time::Instant` panics on
/// wasm targets, so callers must take "now" from here (web-time there)
/// instead of naming `std::time` themselves.
pub(crate) fn scroll_now() -> Instant {
    Instant::now()
}

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
/// mid-gesture). Without this a stale pull would freeze on screen. Generous,
/// because fingers *resting* on the trackpad mid-pull also produce silence —
/// macOS holds the rubber under resting fingers, so the watchdog must only
/// catch genuinely lost releases, not pauses in a live gesture.
const GESTURE_TIMEOUT: Duration = Duration::from_millis(1_200);
/// A precise (trackpad) `Moved` event arriving after a gap this long cannot
/// be momentum — macOS momentum streams tick continuously until they die —
/// so it is a finger resuming contact (its `Started` was consumed by an
/// earlier gesture, or the watchdog released it during a rest). Re-enter
/// position control instead of misreading the finger as momentum impulses.
const STREAM_GAP: Duration = Duration::from_millis(100);
/// Time constant for smooth wheel scrolling; ~90 ms reaches 63% of the
/// remaining distance, fast enough to feel direct.
const SMOOTH_TAU: f32 = 0.09;
/// Distance (px) at which smooth scrolling snaps to its target.
const SMOOTH_EPSILON: f32 = 0.5;
/// Fraction of the viewport the rubber-band asymptotically approaches.
const RUBBER_LIMIT_FRACTION: f32 = 0.45;
/// Floor for the rubber-band limit so tiny panes still visibly pull.
const RUBBER_LIMIT_MIN: f32 = 60.0;
/// Hard ceiling on the rubber-band limit: the displayed overscroll never
/// reaches 150 px no matter how tall the viewport is (the curve is
/// asymptotic, so the shift stays strictly below this).
const RUBBER_LIMIT_MAX: f32 = 150.0;
/// Cap on the raw accumulated pull, ~3× the largest rubber limit (display
/// ≈ 75% of the limit there). Deep into the asymptote the curve's slope
/// collapses, so an uncapped raw pull would make a reversed gesture unwind
/// for hundreds of pixels before the content visibly responds.
const MAX_RAW_PULL: f32 = 450.0;

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
    /// Per axis: the current momentum stream has already met the edge, and
    /// its collision was spent (one spring impulse). Every later momentum
    /// delta that would push past the edge is swallowed — otherwise the
    /// decaying momentum tail re-pumps the spring each event while the
    /// spring fights back, which reads as jitter. Cleared when a new gesture
    /// starts (or a resumed finger re-arms position control).
    momentum_absorbed: (bool, bool),
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
            momentum_absorbed: (false, false),
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

        // Episode start: when the surface was at rest, refresh the step
        // clock so the first animation frame integrates one tick — not the
        // stale gap since the previous episode, which would jolt the spring
        // or glide several frames ahead in a single visible jump.
        if !self.needs_step() {
            self.last_step = Some(now);
        }

        let gap = self.last_event.map(|t| now.saturating_duration_since(t));
        let dt_event = gap
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
                self.momentum_absorbed = (false, false);
                self.smooth_target = None;
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.gesture_active = false;
            }
            _ => {
                // A precise `Moved` after a real pause is a finger resuming
                // contact, not momentum (see `STREAM_GAP`): re-enter
                // position control so the finger drives the pull directly
                // instead of being misread as momentum impulses.
                if !self.gesture_active && gap.is_none_or(|g| g > STREAM_GAP) {
                    self.gesture_active = true;
                    self.velocity = (0.0, 0.0);
                    self.momentum_absorbed = (false, false);
                    self.smooth_target = None;
                }
            }
        }

        let gesture = self.gesture_active;
        let mut out = [scroll.0, scroll.1];
        let axes = [
            (
                0usize,
                scroll.0,
                delta.0,
                max.0,
                &mut self.pull.0,
                &mut self.velocity.0,
                &mut self.momentum_absorbed.0,
            ),
            (
                1usize,
                scroll.1,
                delta.1,
                max.1,
                &mut self.pull.1,
                &mut self.velocity.1,
                &mut self.momentum_absorbed.1,
            ),
        ];
        for (ix, axis_scroll, axis_delta, axis_max, pull, vel, absorbed) in axes {
            if axis_max <= 0.0 {
                // Not scrollable on this axis: no motion, no bounce.
                *pull = 0.0;
                out[ix] = axis_scroll;
                continue;
            }
            out[ix] = if gesture {
                // Position control: the raw pull tracks the fingers 1:1
                // through the clamp, so reversing direction unwinds it
                // before the offset moves again. Capped so the display (long
                // saturated by the rubber curve) responds promptly when the
                // gesture reverses.
                let virtual_offset = axis_scroll - axis_delta + *pull;
                let clamped = virtual_offset.clamp(0.0, axis_max);
                *pull = (virtual_offset - clamped).clamp(-MAX_RAW_PULL, MAX_RAW_PULL);
                clamped
            } else {
                // Momentum: in-range deltas scroll normally. The *first*
                // event of the stream to cross an edge converts the
                // content's impact velocity into one spring impulse; every
                // later edge-crossing delta of the same stream is swallowed
                // (`absorbed`) — re-pumping the spring per event while it
                // fights back reads as jitter. A bounce already in flight
                // (e.g. released mid-pull, tail still arriving) also absorbs
                // the collision without a fresh kick.
                let unclamped = axis_scroll - axis_delta;
                let clamped = unclamped.clamp(0.0, axis_max);
                let excess = unclamped - clamped;
                if excess != 0.0 {
                    if !*absorbed && *pull == 0.0 && *vel == 0.0 {
                        *vel = (-axis_delta / dt_event)
                            .clamp(-MAX_IMPULSE_VELOCITY, MAX_IMPULSE_VELOCITY);
                    }
                    *absorbed = true;
                }
                clamped
            };
        }
        (out[0], out[1])
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
        let limit =
            |dim: f32| (dim * RUBBER_LIMIT_FRACTION).clamp(RUBBER_LIMIT_MIN, RUBBER_LIMIT_MAX);
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
        // No Ended ever arrives. Steps within the timeout hold the pull —
        // resting fingers are a live gesture, so the hold must survive well
        // past a typical pause…
        let (_, active) = p.step(s, max, ms(base, 900));
        assert!(active);
        assert!(p.display_shift((800.0, 600.0)).1 > 0.0);
        // …after the timeout the spring takes over and settles home.
        for i in 0..600u64 {
            let (ns, active) = p.step(s, max, ms(base, 1400 + i * 16));
            s = ns;
            if !active {
                break;
            }
        }
        assert_eq!(p.display_shift((800.0, 600.0)), (0.0, 0.0));
        assert!(!p.needs_step());
    }

    #[test]
    fn displayed_overscroll_never_reaches_150px() {
        let mut p = ScrollPhysics::default();
        let base = t0();
        let max = (0.0, 5_000.0);
        // A giant viewport and an absurdly long pull: raw pull caps at
        // MAX_RAW_PULL and the limit caps at 150, so the display stays
        // strictly under 150 px.
        let mut s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        for i in 1..=50u64 {
            s = p.on_wheel(
                (0.0, 200.0),
                true,
                TouchPhase::Moved,
                s,
                max,
                true,
                ms(base, i * 8),
            );
        }
        let (_, sy) = p.display_shift((3_000.0, 4_000.0));
        assert!(
            sy < 150.0,
            "displayed overscroll must stay under 150, got {sy}"
        );
        assert!(
            sy > 100.0,
            "a full pull should still get near the cap, got {sy}"
        );
    }

    #[test]
    fn raw_pull_cap_keeps_reversal_responsive() {
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
        // Pull 3000 px raw; the cap holds it at MAX_RAW_PULL.
        for i in 1..=30u64 {
            s = p.on_wheel(
                (0.0, 100.0),
                true,
                TouchPhase::Moved,
                s,
                max,
                true,
                ms(base, i * 8),
            );
        }
        let full = p.display_shift((800.0, 600.0)).1;
        // Reversing 300 px must visibly move the display (with an uncapped
        // 3000 px raw pull it would barely change).
        s = p.on_wheel(
            (0.0, -300.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 300),
        );
        assert_eq!(s, (0.0, 0.0), "still inside the pull, offset unmoved");
        let after = p.display_shift((800.0, 600.0)).1;
        assert!(
            after < full - 5.0,
            "300 px of reversal must visibly retract the pull ({full} -> {after})"
        );
    }

    #[test]
    fn momentum_tail_does_not_repump_the_spring() {
        // Two identical runs; one receives a long decaying momentum tail
        // after the first edge collision. The bounce must be identical —
        // the tail is swallowed, not pumped into the spring.
        let base = t0();
        let max = (0.0, 500.0);
        let mut kicked = ScrollPhysics::default();
        let mut control = ScrollPhysics::default();
        for p in [&mut kicked, &mut control] {
            let s = p.on_wheel(
                (0.0, 0.0),
                true,
                TouchPhase::Started,
                (0.0, 490.0),
                max,
                true,
                base,
            );
            let s = p.on_wheel(
                (0.0, -10.0),
                true,
                TouchPhase::Moved,
                s,
                max,
                true,
                ms(base, 8),
            );
            let s = p.on_wheel(
                (0.0, 0.0),
                true,
                TouchPhase::Ended,
                s,
                max,
                true,
                ms(base, 16),
            );
            // First momentum event crosses the edge: one impulse.
            p.on_wheel(
                (0.0, -30.0),
                true,
                TouchPhase::Moved,
                s,
                max,
                true,
                ms(base, 24),
            );
        }
        // Only `kicked` gets the decaying tail, interleaved with steps.
        for i in 1..=10u64 {
            kicked.step((0.0, 500.0), max, ms(base, 24 + i * 16));
            kicked.on_wheel(
                (0.0, -20.0),
                true,
                TouchPhase::Moved,
                (0.0, 500.0),
                max,
                true,
                ms(base, 24 + i * 16 + 4),
            );
            control.step((0.0, 500.0), max, ms(base, 24 + i * 16));
        }
        let shift_kicked = kicked.display_shift((800.0, 600.0)).1;
        let shift_control = control.display_shift((800.0, 600.0)).1;
        assert!(
            (shift_kicked - shift_control).abs() < 0.01,
            "momentum tail altered the bounce: {shift_kicked} vs {shift_control}"
        );
        // And both settle home.
        settle(&mut kicked, (0.0, 500.0), max);
        assert_eq!(kicked.display_shift((800.0, 600.0)), (0.0, 0.0));
    }

    #[test]
    fn flick_release_tail_does_not_fight_the_return_spring() {
        // Release with the rubber held, then let the momentum tail arrive:
        // the tail must not kick the returning spring (that alternating
        // fight is the visible jitter this guards against).
        let base = t0();
        let max = (0.0, 500.0);
        let mut p = ScrollPhysics::default();
        let s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        let s = p.on_wheel(
            (0.0, 80.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        let s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Ended,
            s,
            max,
            true,
            ms(base, 16),
        );
        // Spring starts returning; each tail event would previously inject
        // an opposing impulse. The shift must now decrease monotonically.
        let mut last = p.display_shift((800.0, 600.0)).1;
        let mut scroll = s;
        for i in 1..=30u64 {
            let (ns, active) = p.step(scroll, max, ms(base, 16 + i * 16));
            scroll = ns;
            p.on_wheel(
                (0.0, 25.0),
                true,
                TouchPhase::Moved,
                scroll,
                max,
                true,
                ms(base, 16 + i * 16 + 4),
            );
            let shift = p.display_shift((800.0, 600.0)).1;
            assert!(
                shift <= last + 0.01,
                "return must be monotonic under a momentum tail ({last} -> {shift})"
            );
            last = shift;
            if !active {
                break;
            }
        }
        // Tail over; the spring finishes the return unassisted.
        settle(&mut p, scroll, max);
        assert_eq!(p.display_shift((800.0, 600.0)), (0.0, 0.0));
    }

    #[test]
    fn resumed_finger_after_watchdog_release_regains_position_control() {
        let base = t0();
        let max = (0.0, 500.0);
        let mut p = ScrollPhysics::default();
        let s = p.on_wheel(
            (0.0, 0.0),
            true,
            TouchPhase::Started,
            (0.0, 0.0),
            max,
            true,
            base,
        );
        let s = p.on_wheel(
            (0.0, 90.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 8),
        );
        // Fingers rest past the watchdog; the spring reclaims some pull.
        p.step(s, max, ms(base, 1_300));
        p.step(s, max, ms(base, 1_316));
        // Fingers move again — `Moved` after a long gap, no `Started`. This
        // must re-enter position control (a further pull deepens the shift,
        // it is not misread as a momentum impulse to swallow).
        let before = p.display_shift((800.0, 600.0)).1;
        let s2 = p.on_wheel(
            (0.0, 40.0),
            true,
            TouchPhase::Moved,
            s,
            max,
            true,
            ms(base, 1_400),
        );
        assert_eq!(s2, (0.0, 0.0));
        let after = p.display_shift((800.0, 600.0)).1;
        assert!(
            after > before,
            "resumed finger must deepen the pull directly ({before} -> {after})"
        );
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

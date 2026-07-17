//! The datatable's motion layer — one small, coherent, opt-out vocabulary of
//! entrance transitions shared by the flat grid and the pivot.
//!
//! # What moves, and what deliberately does not
//!
//! The data surface itself — cells, selection, hover, sort state — is painted
//! imperatively every frame and stays **instant**. That is a design choice, not
//! a gap: a spreadsheet's value is that state reads the very frame it changes,
//! and Excel / Numbers never tween a selection marquee. Motion here is reserved
//! for the *chrome* that pops into existence — context menus, filter panels,
//! popovers, dialogs, the busy scrim, the drag ghost — where an instant appear
//! reads as a jarring flash and a short fade reads as native.
//!
//! # The vocabulary
//!
//! A single gesture: an opacity fade on enter, fast (~110 ms) and eased with
//! [`ease_out_quint`] so it decelerates into place. No scale, no bounce, no
//! slide — those betray a web toolkit; macOS menus simply fade, so we do too.
//! Exits are instant (the element is dropped from the tree), matching the
//! platform. Every surface uses [`pop_in`], so the whole crate speaks one
//! motion dialect rather than a dozen hand-tuned ones.
//!
//! # Opting out
//!
//! All motion routes through the `animations` flag ([`crate::GridConfig`],
//! mirrored onto [`crate::pivot::PivotState`]). GPUI 0.2 exposes no OS
//! reduce-motion signal, so this flag *is* the accessibility control: a host
//! that honors a system "reduce motion" preference sets it to `false` and every
//! surface falls back to an instant appear, with zero other changes.

use std::time::Duration;

use gpui::{ease_out_quint, Animation, AnimationExt, ElementId, IntoElement, Styled};

/// Enter duration for transient surfaces (menus, popovers, dialogs). Fast
/// enough to feel instant to a user in a task (product-register motion budget
/// is 150–250 ms; a reveal wants the low end), slow enough to register as a
/// fade rather than a pop.
pub(crate) const SURFACE_ENTER_MS: u64 = 110;

/// Enter duration for the busy scrim. A touch slower than a menu: the overlay
/// covers the whole grid, and a softer fade signals "work is starting" instead
/// of snapping a wall in front of the data.
pub(crate) const SCRIM_ENTER_MS: u64 = 150;

/// Enter duration for the pivot drag ghost. Quicker than a menu — the ghost is
/// a direct extension of the pointer, so it should feel lifted, not revealed.
pub(crate) const GHOST_ENTER_MS: u64 = 90;

/// Enter duration for a sidebar accordion body on expand. Sits with the other
/// state transitions; the header chevron flips instantly, the body fades in.
pub(crate) const SECTION_ENTER_MS: u64 = 140;

/// Fade `element` in over `duration_ms`, easing out. When `animations` is
/// `false` the element is returned untouched (instant appear) so the same call
/// site serves both the animated and the reduce-motion path.
///
/// Apply this to the *visible, styled* body of a surface — the card with the
/// background and border — not to a wrapping `anchored`/`deferred`/`occlude`
/// layer (those are not [`Styled`], and fading an invisible click-catcher would
/// do nothing). Opacity does not affect layout, so an `anchored()` parent keeps
/// its edge-flip positioning stable across the fade.
pub(crate) fn fade_in<E>(
    element: E,
    id: impl Into<ElementId>,
    duration_ms: u64,
    animations: bool,
) -> gpui::AnyElement
where
    E: Styled + IntoElement + 'static,
{
    if !animations {
        return element.into_any_element();
    }
    element
        .with_animation(
            id,
            Animation::new(Duration::from_millis(duration_ms)).with_easing(ease_out_quint()),
            |el, delta| el.opacity(delta),
        )
        .into_any_element()
}

/// [`fade_in`] at the standard transient-surface duration ([`SURFACE_ENTER_MS`]).
/// The default entrance for every menu, popover, and dialog in the crate.
pub(crate) fn pop_in<E>(element: E, id: impl Into<ElementId>, animations: bool) -> gpui::AnyElement
where
    E: Styled + IntoElement + 'static,
{
    fade_in(element, id, SURFACE_ENTER_MS, animations)
}

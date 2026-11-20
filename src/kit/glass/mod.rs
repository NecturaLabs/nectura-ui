//! Translucent surfaces: the safe facade over the backdrop blur (design spec 7).
//!
//! Floating layers (menus, select lists, popovers, toasts, dialog cards, palette, cheatsheet) ask
//! [`layer_fill`] for their fill. With Translucent surfaces on and the blur available, it adds two
//! OpenGL paint callbacks under the layer: the capture, which copies and blurs what is already
//! painted behind it, and the composite, which draws that copy inside the layer with its
//! luminance clamped so the theme's text stays readable. It returns the overlay at the theme's
//! glass opacity; otherwise it returns the opaque overlay, the fallback that keeps the same
//! layout. [`paint::surface`](super::paint::surface) moves the composite between the layer's
//! shadow and its fill, and the capture stays before the shadow, so the shadow never shows
//! through the glass.
//!
//! The renderer is `gl.rs`, the only module allowed to call OpenGL, and the only code in Canopy
//! the compiler lets use raw OpenGL calls; this file decides what to draw and keeps every number
//! it hands over in range. Whatever fails there (an old driver, a shader the driver rejects, an
//! OpenGL error, a lost context, a panic) turns the blur off for the session, logs once, covers
//! that layer with the opaque overlay for the frame in which it failed, and makes the setting
//! say "Not supported by this graphics driver". Nothing loops: the blur runs only in frames egui
//! paints anyway, for at most [`MAX_REGIONS`] layers a frame.

mod gl;
mod shaders;

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui_glow;
use eframe::glow::{self, HasContext as _};
use egui::layers::ShapeIdx;
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, LayerId, PaintCallback, Painter, Rect, RichText,
    Sense, Shape, Ui, pos2, vec2,
};

use crate::theme::{self, Elevation, Fill, Palette};

/// How far the blur reaches beyond a layer, in points: three standard deviations of the
/// 12-point blur, so the edge of the layer samples real content.
const PAD: f32 = 36.0;
/// Glass layers per frame; more fall back to the opaque overlay for that frame (design spec 7.1).
pub const MAX_REGIONS: u8 = 4;
/// Regions narrower or shorter than this many pixels are not worth blurring.
const MIN_SIDE: i32 = 8;
/// The largest texture side the blur uses, whatever the driver allows.
const MAX_SIDE_CAP: i32 = 4096;
/// The smallest maximum texture size the blur accepts from a driver.
const MIN_TEXTURE_SIDE: i32 = 256;
/// Textures grow in steps of this many pixels, so menus of slightly different sizes reuse them.
const GROWTH: i32 = 256;
/// Down passes at most, and the textures that takes (the half-size copy plus one per pass).
const MAX_PASSES: usize = 4;
const LEVELS: usize = MAX_PASSES + 1;
/// Saturation of the blurred backdrop, mixed from its Rec. 709 luma.
const SATURATION: f32 = 1.15;
/// Largest coordinate, in pixels, the blur works with; anything beyond is a broken input.
const COORD_LIMIT: f32 = 16_777_216.0;

// ------------------------------------------------------------------------------------- state

/// Where the blur stands: not probed yet (no OpenGL context seen), supported (the context is new
/// enough; nothing is built until a layer needs it), built, or off for the rest of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Unprobed,
    Supported,
    Ready,
    Unavailable,
}

impl Backend {
    fn code(self) -> u8 {
        match self {
            Self::Unprobed => 0,
            Self::Supported => 1,
            Self::Ready => 2,
            Self::Unavailable => 3,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Supported,
            2 => Self::Ready,
            3 => Self::Unavailable,
            _ => Self::Unprobed,
        }
    }
}

/// Why the blur is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reason {
    /// No OpenGL context (another renderer).
    NoContext,
    /// Older than desktop OpenGL 3.0 or OpenGL ES 3.0.
    OldGl,
    /// The window's framebuffer is multisampled, which a scaled blit cannot read.
    Multisampled,
    /// The driver's largest texture is under `MIN_TEXTURE_SIDE`.
    SmallTextures,
    /// The driver rejected a shader.
    Compile,
    /// The driver could not link a program.
    Link,
    /// A program lacks a uniform it must have.
    Uniform,
    /// A blur texture could not be attached to a framebuffer.
    Framebuffer,
    /// OpenGL reported an error after the blur's own calls.
    Gl(u32),
    /// The OpenGL context was lost (driver reset).
    ContextLost,
    /// A panic inside the renderer, caught before it reached egui.
    Panic,
    /// The renderer's lock was poisoned by an earlier panic.
    Poisoned,
    /// The window closed and the blur's objects were deleted.
    Exited,
    /// An internal inconsistency (missing textures); never expected.
    Internal,
}

impl Reason {
    fn code(self) -> u8 {
        match self {
            Self::NoContext => 1,
            Self::OldGl => 2,
            Self::Multisampled => 3,
            Self::SmallTextures => 4,
            Self::Compile => 5,
            Self::Link => 6,
            Self::Uniform => 7,
            Self::Framebuffer => 8,
            Self::Gl(_) => 9,
            Self::ContextLost => 10,
            Self::Panic => 11,
            Self::Poisoned => 12,
            Self::Exited => 13,
            Self::Internal => 14,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::NoContext,
            2 => Self::OldGl,
            3 => Self::Multisampled,
            4 => Self::SmallTextures,
            5 => Self::Compile,
            6 => Self::Link,
            7 => Self::Uniform,
            8 => Self::Framebuffer,
            9 => Self::Gl(0),
            10 => Self::ContextLost,
            11 => Self::Panic,
            12 => Self::Poisoned,
            13 => Self::Exited,
            14 => Self::Internal,
            _ => return None,
        })
    }

    fn describe(self) -> String {
        match self {
            Self::NoContext => "no OpenGL context".into(),
            Self::OldGl => "OpenGL older than 3.0 (or OpenGL ES 3.0)".into(),
            Self::Multisampled => "the window framebuffer is multisampled".into(),
            Self::SmallTextures => "the driver's textures are too small".into(),
            Self::Compile => "the driver rejected a blur shader".into(),
            Self::Link => "the driver could not link a blur program".into(),
            Self::Uniform => "a blur program lacks a uniform".into(),
            Self::Framebuffer => "a blur framebuffer is incomplete".into(),
            Self::Gl(0) => "OpenGL error".into(),
            Self::Gl(code) => format!("OpenGL error 0x{code:04X}"),
            Self::ContextLost => "the OpenGL context was lost".into(),
            Self::Panic => "the blur renderer panicked".into(),
            Self::Poisoned => "the blur renderer's lock is poisoned".into(),
            Self::Exited => "the window closed".into(),
            Self::Internal => "internal error".into(),
        }
    }
}

/// The blur's state, shared by the interface and the paint callbacks.
struct Status {
    backend: AtomicU8,
    reason: AtomicU8,
    max_side: AtomicI32,
}

impl Status {
    const fn new() -> Self {
        Self {
            backend: AtomicU8::new(0),
            reason: AtomicU8::new(0),
            max_side: AtomicI32::new(MAX_SIDE_CAP),
        }
    }
}

#[cfg(not(any(test, feature = "testkit")))]
static STATUS: Status = Status::new();

#[cfg(not(any(test, feature = "testkit")))]
fn status<R>(f: impl FnOnce(&Status) -> R) -> R {
    f(&STATUS)
}

// Tests run in parallel threads, each with its own state.
#[cfg(any(test, feature = "testkit"))]
thread_local! {
    static STATUS: Status = const { Status::new() };
}

#[cfg(any(test, feature = "testkit"))]
fn status<R>(f: impl FnOnce(&Status) -> R) -> R {
    STATUS.with(f)
}

fn backend() -> Backend {
    Backend::from_code(status(|s| s.backend.load(Ordering::Relaxed)))
}

/// Moves `from` to `to` if the blur is still in `from`.
fn advance(from: Backend, to: Backend) -> bool {
    status(|s| {
        s.backend
            .compare_exchange(from.code(), to.code(), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    })
}

/// Turns the blur off for the session. Returns whether this call did (the first failure logs).
fn latch(reason: Reason) -> bool {
    let before = status(|s| {
        s.backend
            .swap(Backend::Unavailable.code(), Ordering::Relaxed)
    });
    if Backend::from_code(before) == Backend::Unavailable {
        return false;
    }
    status(|s| s.reason.store(reason.code(), Ordering::Relaxed));
    if reason == Reason::Exited {
        tracing::debug!("translucent surfaces: the blur's objects were deleted at exit");
    } else {
        tracing::warn!(
            "translucent surfaces are unavailable: {}; floating layers are opaque",
            reason.describe()
        );
    }
    true
}

/// The largest region side the blur may use: the driver's texture limit once known.
fn max_side() -> i32 {
    status(|s| s.max_side.load(Ordering::Relaxed))
}

fn set_max_side(side: i32) {
    status(|s| {
        s.max_side
            .store(side.clamp(0, MAX_SIDE_CAP), Ordering::Relaxed);
    });
}

/// Numbers the frame's glass layers, so a composite knows whether the levels hold its capture.
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

/// OpenGL objects the renderer created and deleted, for the soak check.
static CREATED: AtomicU64 = AtomicU64::new(0);
static DELETED: AtomicU64 = AtomicU64::new(0);

/// The renderer: its programs, textures and framebuffers once built. Locked only while painting
/// and at exit, both on the thread that owns the OpenGL context.
static SLOT: Mutex<gl::Slot> = Mutex::new(gl::Slot::Empty);

/// Records whether the window's OpenGL context can run the blur (`cc.gl` at startup). Builds
/// nothing: the shaders compile when the first glass layer is painted.
pub fn init(gl: Option<&glow::Context>) {
    let Some(gl) = gl else {
        latch(Reason::NoContext);
        return;
    };
    let v = gl.version();
    if shaders::header(v.major, v.minor, v.is_embedded).is_some() {
        advance(Backend::Unprobed, Backend::Supported);
    } else {
        latch(Reason::OldGl);
    }
}

/// Whether this build and graphics backend can blur the backdrop (it may still fail when it is
/// first built, which turns it off).
pub fn available() -> bool {
    matches!(backend(), Backend::Supported | Backend::Ready)
}

/// The blur's state in words, for diagnostics.
pub fn describe() -> String {
    match backend() {
        Backend::Unprobed => "not probed".into(),
        Backend::Supported => "supported, not built yet".into(),
        Backend::Ready => "ready".into(),
        Backend::Unavailable => format!(
            "unavailable ({})",
            Reason::from_code(status(|s| s.reason.load(Ordering::Relaxed)))
                .map_or_else(|| "unknown".into(), Reason::describe)
        ),
    }
}

/// OpenGL objects the blur has created and deleted so far.
pub fn counters() -> (u64, u64) {
    (
        CREATED.load(Ordering::Relaxed),
        DELETED.load(Ordering::Relaxed),
    )
}

/// The graphics context eframe hands to `App::on_exit`, and the only thing [`destroy`] accepts.
///
/// Deleting the blur's OpenGL objects is sound only while the context they were made in is
/// current on the calling thread. eframe guarantees that in `App::on_exit` and nowhere else the
/// application controls: it keeps the context current there to delete its own objects right after.
/// Safe Rust cannot check the condition, so the function that relies on it asks for this token
/// instead of a bare context, which puts the condition in the type and at the call site.
///
/// Make one with [`ExitContext::from_on_exit`], inside `App::on_exit`, from the argument eframe
/// passes, and nowhere else. The token is not `Clone` or `Copy`: [`destroy`] consumes it, so one
/// exit deletes once. Whatever the caller does, `destroy` still makes OpenGL calls only for the
/// context the objects were made in and on the thread that made them.
///
/// Without a token, `destroy` does not compile:
///
/// ```compile_fail,E0308
/// use nectura_ui::kit::glass;
/// glass::destroy(None);
/// ```
pub struct ExitContext<'a> {
    gl: Option<&'a glow::Context>,
}

impl<'a> ExitContext<'a> {
    /// The token for the context `App::on_exit` received (`None` when eframe passes none, as
    /// with a non-OpenGL backend).
    ///
    /// Call it only inside `App::on_exit` with that argument. Called anywhere else it is
    /// harmless but leaks: the blur's objects are left to their context.
    #[must_use]
    pub const fn from_on_exit(gl: Option<&'a glow::Context>) -> Self {
        Self { gl }
    }
}

impl std::fmt::Debug for ExitContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExitContext")
            .field("has_context", &self.gl.is_some())
            .finish()
    }
}

/// Deletes the blur's OpenGL objects and turns the blur off for good.
///
/// Call it from `App::on_exit` with `ExitContext::from_on_exit(gl)`, before eframe deletes its
/// own objects; see [`ExitContext`] for why. Elsewhere it is harmless but leaks: the objects are
/// deleted only for the context they were made in and on the thread that made them; any other
/// call makes no OpenGL call and leaves them to their context. A second call does nothing.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the token is spent: a second exit needs a second token"
)]
pub fn destroy(exit: ExitContext<'_>) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut slot) = SLOT.lock() {
            gl::destroy(&mut slot, exit.gl);
        } else {
            tracing::debug!("translucent surfaces: lock poisoned at exit; objects left");
        }
    }));
    if result.is_err() {
        tracing::warn!("translucent surfaces: panic while deleting the blur's objects");
    }
    latch(Reason::Exited);
}

// --------------------------------------------------------------- system reduced transparency

/// What the operating system said: 0 not known yet, 1 transparency allowed, 2 reduced.
static SYSTEM: AtomicU8 = AtomicU8::new(0);
static QUERYING: AtomicBool = AtomicBool::new(false);

/// The operating system's reduce-transparency answer, once it has arrived (`None` while unknown,
/// or where the system has no such setting, as on Linux).
pub fn reduced_transparency() -> Option<bool> {
    match SYSTEM.load(Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

/// Asks the operating system for its reduce-transparency setting on a background thread (the
/// query runs a helper program) and repaints when it answers, so the app re-applies the theme.
pub fn query_system(ctx: &egui::Context) {
    if QUERYING.swap(true, Ordering::Relaxed) {
        return;
    }
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("transparency-query".into())
        .spawn(move || {
            let answer =
                crate::host::get().and_then(<dyn crate::host::Host>::prefers_reduced_transparency);
            SYSTEM.store(
                match answer {
                    Some(true) => 2,
                    Some(false) => 1,
                    None => 0,
                },
                Ordering::Relaxed,
            );
            QUERYING.store(false, Ordering::Relaxed);
            if answer.is_some() {
                ctx.request_repaint();
            }
        });
    if let Err(e) = spawned {
        QUERYING.store(false, Ordering::Relaxed);
        tracing::warn!("could not ask the system about reduced transparency: {e}");
    }
}

// ------------------------------------------------------------------------------------ facade

/// The fill a floating layer paints at `rect`: the overlay at the theme's glass opacity over the
/// blurred backdrop when glass is on, else the opaque overlay. `rect` must be the rectangle the
/// layer paints (with its entrance transform applied). Call before the layer's surface; when that
/// surface is painted with `kit::paint::surface` at the same `rect`, the backdrop moves between
/// its shadow and its fill.
pub fn layer_fill(painter: &Painter, rect: Rect, radius: CornerRadius) -> Color32 {
    layer_fill_with(painter, rect, radius, theme::appearance().glass)
}

/// The paint callbacks of one glass layer, for a caller that places them itself.
#[derive(Debug)]
pub struct Backdrop {
    /// Copies and blurs what is behind the layer: place it before the layer's shadow.
    pub capture: Shape,
    /// Draws the blurred copy inside the layer: place it after the shadow, before the fill.
    pub composite: Shape,
}

/// The callbacks for a layer at `rect`, for a caller that places them itself (a reserved shape
/// index under an egui `Frame`), or `None` when the layer is opaque. Counts towards the frame's
/// glass layers only when it returns them. With the capture before the shadow and the composite
/// after it, the shadow stays outside the glass.
pub fn backdrop_shapes(painter: &Painter, rect: Rect, radius: CornerRadius) -> Option<Backdrop> {
    backdrop_shapes_with(painter, rect, radius, theme::appearance().glass)
}

/// The translucent overlay painted over a blurred backdrop, leaning to the theme's light.
pub fn glass_fill(ctx: &egui::Context) -> Color32 {
    let p = theme::palette(ctx);
    p.glass(theme::surfaces(ctx))
}

fn layer_fill_with(painter: &Painter, rect: Rect, radius: CornerRadius, wanted: bool) -> Color32 {
    if backdrop_with(painter, rect, radius, wanted) {
        glass_fill(painter.ctx())
    } else {
        theme::surfaces(painter.ctx()).overlay
    }
}

fn backdrop_with(painter: &Painter, rect: Rect, radius: CornerRadius, wanted: bool) -> bool {
    let Some(Backdrop { capture, composite }) = backdrop_shapes_with(painter, rect, radius, wanted)
    else {
        return false;
    };
    painter.add(capture);
    let idx = painter.add(composite.clone());
    remember(painter, rect, idx, composite);
    true
}

fn backdrop_shapes_with(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    wanted: bool,
) -> Option<Backdrop> {
    if !wanted || !available() {
        return None;
    }
    let ctx = painter.ctx();
    let p = theme::palette(ctx);
    if p.glass_alpha == 0 || !painter.is_visible() || painter.opacity() <= 0.0 {
        return None;
    }
    let clip = painter.clip_rect();
    if !rect.is_finite() || !rect.is_positive() || !clip.intersects(rect) {
        return None;
    }
    let ppp = ctx.pixels_per_point();
    let viewport = ctx.viewport_rect();
    let screen = screen_px(viewport, ppp)?;
    plan(rect, radius, ppp, screen, clip, max_side())?.blur?;
    if !take_budget(ctx) {
        return None;
    }
    let job = Arc::new(Job {
        id: NEXT_JOB.fetch_add(1, Ordering::Relaxed),
        layer: rect,
        radius,
        look: Look {
            dark: p.dark,
            cap: p.glass_luminance.clamp(0.001, 0.999),
            opacity: painter.opacity().clamp(0.0, 1.0),
        },
        fallback: theme::surfaces(ctx).overlay,
        ctx: ctx.clone(),
    });
    let area = rect.expand(PAD).intersect(viewport);
    let callback = |step: fn(&Job) -> gl::Step| {
        let job = Arc::clone(&job);
        Shape::Callback(PaintCallback {
            rect: area,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                run(&info, painter, &job, step(&job));
            })),
        })
    };
    Some(Backdrop {
        capture: callback(|job| gl::Step::Capture { job: job.id }),
        composite: callback(|job| gl::Step::Composite {
            job: job.id,
            look: job.look,
            fallback: job.fallback,
        }),
    })
}

/// The window size in pixels, as eframe paints it.
fn screen_px(viewport: Rect, ppp: f32) -> Option<[u32; 2]> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the cast saturates, and its value is kept only when within 1..COORD_LIMIT"
    )]
    let side = |points: f32| {
        let px = (points * ppp).round();
        (1.0..COORD_LIMIT).contains(&px).then_some(px as u32)
    };
    Some([side(viewport.width())?, side(viewport.height())?])
}

/// Takes one of the frame's glass layers, if any is left.
fn take_budget(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let used = d.get_temp_mut_or_default::<(u64, u8)>(Id::new("canopy-glass-budget"));
        if used.0 != pass {
            *used = (pass, 0);
        }
        if used.1 >= MAX_REGIONS {
            false
        } else {
            used.1 += 1;
            true
        }
    })
}

/// A backdrop added by [`layer_fill`] that the layer's surface may still move.
#[derive(Clone)]
struct Pending {
    pass: u64,
    layer: LayerId,
    rect: Rect,
    idx: ShapeIdx,
    shape: Shape,
}

fn pending_id() -> Id {
    Id::new("canopy-glass-pending")
}

fn remember(painter: &Painter, rect: Rect, idx: ShapeIdx, shape: Shape) {
    let pass = painter.ctx().cumulative_pass_nr();
    let layer = painter.layer_id();
    painter.ctx().data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Vec<Pending>>(pending_id());
        list.retain(|p| p.pass == pass);
        list.push(Pending {
            pass,
            layer,
            rect,
            idx,
            shape,
        });
    });
}

/// Takes back the backdrop [`layer_fill`] added for a layer at `rect` on this painter's layer in
/// this pass, leaving nothing in its old place, so `kit::paint::surface` can add it between the
/// layer's shadow and its fill (design spec 7.4).
pub(crate) fn lift(painter: &Painter, rect: Rect) -> Option<Shape> {
    if !painter.is_visible() {
        return None;
    }
    let pass = painter.ctx().cumulative_pass_nr();
    let layer = painter.layer_id();
    let pending = painter.ctx().data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Vec<Pending>>(pending_id());
        list.retain(|p| p.pass == pass);
        let i = list
            .iter()
            .position(|p| p.layer == layer && p.rect == rect)?;
        Some(list.swap_remove(i))
    })?;
    painter.set(pending.idx, Shape::Noop);
    Some(pending.shape)
}

// ------------------------------------------------------------------------------ paint callback

/// What the composite needs from the theme and the layer.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    dark: bool,
    /// `glass_luminance`: the clamp's cap, kept strictly between 0 and 1.
    cap: f32,
    /// The layer's opacity (egui's fade-in), applied to the backdrop as to the fill.
    opacity: f32,
}

impl Look {
    /// The composite's `u_look` uniform.
    fn uniform(self) -> [f32; 4] {
        [
            SATURATION,
            self.cap,
            if self.dark { 1.0 } else { 0.0 },
            self.opacity,
        ]
    }
}

/// One glass layer, as its paint callbacks see it.
struct Job {
    /// Unique per layer and frame: ties the composite to its capture.
    id: u64,
    layer: Rect,
    radius: CornerRadius,
    look: Look,
    /// The opaque overlay, painted instead when the blur fails during this paint.
    fallback: Color32,
    ctx: egui::Context,
}

/// A paint callback: never panics into egui_glow, and turns the blur off on any failure. A failed
/// capture leaves the layer to its composite, which then covers it with the opaque overlay.
fn run(
    info: &egui::epaint::PaintCallbackInfo,
    painter: &egui_glow::Painter,
    job: &Job,
    step: gl::Step,
) {
    let Some(plan) = plan(
        job.layer,
        job.radius,
        info.pixels_per_point,
        info.screen_size_px,
        info.clip_rect,
        max_side(),
    ) else {
        return;
    };
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| match SLOT.lock() {
        Ok(mut slot) => gl::paint(&mut slot, painter, &plan, step),
        Err(_) => gl::Outcome::Failed {
            reason: Reason::Poisoned,
            covered: false,
        },
    }))
    .unwrap_or(gl::Outcome::Failed {
        reason: Reason::Panic,
        covered: false,
    });
    match outcome {
        gl::Outcome::Drawn { built } => {
            if let Some(side) = built {
                set_max_side(side);
                advance(Backend::Supported, Backend::Ready);
            }
        }
        gl::Outcome::Covered => {}
        gl::Outcome::Failed { reason, covered } => {
            let composite = matches!(step, gl::Step::Composite { .. });
            if composite && !covered && reason != Reason::ContextLost {
                // The fallback restores what it touches; a panic here is caught and dropped.
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    gl::fallback(painter, &plan, job.fallback);
                }));
            }
            if latch(reason) {
                job.ctx.request_repaint();
            }
        }
    }
}

// ------------------------------------------------------------------------------------- maths

/// A rectangle in window pixels with the origin at the bottom left, as OpenGL counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PxRect {
    /// From exclusive edges; `None` when empty or out of range.
    fn from_edges(x0: i32, y0: i32, x1: i32, y1: i32) -> Option<Self> {
        let w = x1.checked_sub(x0)?;
        let h = y1.checked_sub(y0)?;
        (w > 0 && h > 0).then_some(Self { x: x0, y: y0, w, h })
    }

    fn right(self) -> Option<i32> {
        self.x.checked_add(self.w)
    }

    fn top(self) -> Option<i32> {
        self.y.checked_add(self.h)
    }

    fn intersect(self, other: Self) -> Option<Self> {
        Self::from_edges(
            self.x.max(other.x),
            self.y.max(other.y),
            self.right()?.min(other.right()?),
            self.top()?.min(other.top()?),
        )
    }

    /// Shrunk by `dx` at the left and right and `dy` at the bottom and top.
    fn shrink(self, dx: i32, dy: i32) -> Option<Self> {
        Self::from_edges(
            self.x.checked_add(dx)?,
            self.y.checked_add(dy)?,
            self.right()?.checked_sub(dx)?,
            self.top()?.checked_sub(dy)?,
        )
    }
}

/// The blur of one layer: the window region copied and how it is blurred.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Blur {
    pub region: PxRect,
    pub passes: usize,
    pub offset: f32,
}

/// Everything one paint of a glass layer needs, in window pixels, validated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Plan {
    /// The layer's pixel box on the screen (the composite's viewport).
    pub bounds: PxRect,
    /// The part of `bounds` inside the clip rectangle (the composite's scissor).
    pub scissor: PxRect,
    /// The layer's centre and half size, y up.
    pub rect: [f32; 4],
    /// Corner radii: top left, top right, bottom left, bottom right.
    pub radii: [f32; 4],
    /// `None` when the region is too small or too large to blur: the layer is covered opaque.
    pub blur: Option<Blur>,
}

impl Plan {
    /// The opaque fallback as two scissor rectangles: the layer without its left and right
    /// corner columns, and without its top and bottom corner rows, so no square corner shows
    /// outside the rounded layer.
    pub(super) fn cross(&self) -> [Option<PxRect>; 2] {
        let r = self.radii.iter().fold(0.0_f32, |a, &b| a.max(b)).ceil();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "finite, whole and below COORD_LIMIT, checked first"
        )]
        let r = if r.is_finite() && r < COORD_LIMIT {
            r as i32
        } else {
            0
        };
        [
            self.bounds
                .shrink(r, 0)
                .and_then(|b| b.intersect(self.scissor)),
            self.bounds
                .shrink(0, r)
                .and_then(|b| b.intersect(self.scissor)),
        ]
    }
}

/// `v` points in pixels, rounded by `round`, if finite and in range.
#[expect(
    clippy::cast_possible_truncation,
    reason = "finite, whole and within COORD_LIMIT, checked first"
)]
fn px(v: f32, ppp: f32, round: fn(f32) -> f32) -> Option<i32> {
    let p = round(v * ppp);
    (p.is_finite() && p.abs() < COORD_LIMIT).then_some(p as i32)
}

/// Works out one paint of a layer at `layer` (points) with corner `radius`, on a screen of
/// `screen` pixels at `ppp`, clipped to `clip` (points), with textures of at most `max_side`.
/// `None` when nothing of the layer is visible or an input is out of range.
pub(super) fn plan(
    layer: Rect,
    radius: CornerRadius,
    ppp: f32,
    screen: [u32; 2],
    clip: Rect,
    max_side: i32,
) -> Option<Plan> {
    if !(ppp.is_finite() && (0.25..=16.0).contains(&ppp)) {
        return None;
    }
    if !layer.is_finite() || !layer.is_positive() || !clip.is_finite() {
        return None;
    }
    let sw = i32::try_from(screen[0]).ok()?;
    let sh = i32::try_from(screen[1]).ok()?;
    let screen_box = PxRect::from_edges(0, 0, sw, sh)?;
    // Edges with the origin at the top left, the layer rounded outwards and the clip as egui's
    // scissor rounds it.
    let (lx0, ly0) = (
        px(layer.min.x, ppp, f32::floor)?,
        px(layer.min.y, ppp, f32::floor)?,
    );
    let (lx1, ly1) = (
        px(layer.max.x, ppp, f32::ceil)?,
        px(layer.max.y, ppp, f32::ceil)?,
    );
    let (cx0, cy0) = (
        px(clip.min.x, ppp, f32::round)?,
        px(clip.min.y, ppp, f32::round)?,
    );
    let (cx1, cy1) = (
        px(clip.max.x, ppp, f32::round)?,
        px(clip.max.y, ppp, f32::round)?,
    );
    // Flipped to OpenGL's bottom-left origin.
    let flip = |top: i32, bottom: i32| Some((sh.checked_sub(bottom)?, sh.checked_sub(top)?));
    let (by0, by1) = flip(ly0, ly1)?;
    let bounds = PxRect::from_edges(lx0, by0, lx1, by1)?.intersect(screen_box)?;
    let (ky0, ky1) = flip(cy0, cy1)?;
    let scissor = PxRect::from_edges(cx0, ky0, cx1, ky1)?.intersect(bounds)?;

    let pad = px(PAD, ppp, f32::ceil)?;
    let region = PxRect::from_edges(
        lx0.checked_sub(pad)?,
        by0.checked_sub(pad)?,
        lx1.checked_add(pad)?,
        by1.checked_add(pad)?,
    )?
    .intersect(screen_box)?;
    let blur = (region.w >= MIN_SIDE
        && region.h >= MIN_SIDE
        && region.w <= max_side
        && region.h <= max_side)
        .then(|| {
            let (passes, offset) = kawase(ppp);
            Blur {
                region,
                passes,
                offset,
            }
        });

    let half = [layer.width() * 0.5 * ppp, layer.height() * 0.5 * ppp];
    let centre = layer.center();
    let rect = [
        centre.x * ppp,
        crate::int_f32(sh) - centre.y * ppp,
        half[0],
        half[1],
    ];
    let most = half[0].min(half[1]).max(0.0);
    let r = |c: u8| (f32::from(c) * ppp).min(most);
    Some(Plan {
        bounds,
        scissor,
        rect,
        radii: [r(radius.nw), r(radius.ne), r(radius.sw), r(radius.se)],
        blur,
    })
}

/// Down passes and sample offset (in source texels) that make the dual-Kawase blur a Gaussian of
/// about 12 points at `ppp` (measured by simulating the passes on an impulse): three passes up
/// to 2 pixels per point, four from there, the offset growing with the pixel density between.
pub(super) fn kawase(ppp: f32) -> (usize, f32) {
    let ppp = if ppp.is_finite() {
        ppp.clamp(0.25, 16.0)
    } else {
        1.0
    };
    let (passes, scale) = if ppp >= 2.0 { (4, 0.5) } else { (3, 1.0) };
    let s = ppp * scale;
    (passes.min(MAX_PASSES), (0.572 * s - 0.143).clamp(0.25, 1.2))
}

/// The size used in each blur level for a region: half the region (rounded up) for the copy,
/// half again for each pass, never under one pixel.
pub(super) fn level_sizes(region: [i32; 2]) -> [[i32; 2]; LEVELS] {
    let half = |v: i32| (v / 2 + v % 2).max(1);
    let mut out = [[1, 1]; LEVELS];
    let mut cur = region;
    for level in &mut out {
        cur = [half(cur[0]), half(cur[1])];
        *level = cur;
    }
    out
}

/// The region size the textures must hold for `needed`, given what they hold now: unchanged
/// when it fits, else grown to the next multiple of `GROWTH` (never shrunk) and capped at
/// `max_side`. `None` when `needed` is empty or larger than `max_side`.
pub(super) fn grow(needed: [i32; 2], have: [i32; 2], max_side: i32) -> Option<[i32; 2]> {
    if needed.iter().any(|&n| n < 1 || n > max_side) {
        return None;
    }
    if needed[0] <= have[0] && needed[1] <= have[1] {
        return Some(have);
    }
    let step = |n: i32, h: i32| {
        let steps = n / GROWTH + i32::from(n % GROWTH != 0);
        let up = steps.checked_mul(GROWTH).unwrap_or(max_side);
        up.max(h).min(max_side)
    };
    Some([step(needed[0], have[0]), step(needed[1], have[1])])
}

// -------------------------------------------------------------------------------------- soak

/// The blur soak (`--glass-soak N`, debug builds): every frame opens zero to five floating layers
/// of changing sizes (five is over the frame's limit of glass layers), so the limit, the lift
/// under the shadow and the fallbacks all run. They are glass when Translucent surfaces is on;
/// with it off the same run paints them opaque, the baseline for memory. A warm-up comes first,
/// unmeasured, then N measured frames; [`Soak::report`] gives the object counters and the
/// resident memory at the start and the end of the measured frames.
#[derive(Debug, Clone, Copy)]
pub struct Soak {
    warmup: u64,
    frames: u64,
    frame: u64,
    rss_start: Option<u64>,
    rss_end: Option<u64>,
}

impl Soak {
    /// Frames after which the layers repeat: by then every size and position has been painted,
    /// so the blur's textures and the driver's high-water marks have reached their final size.
    pub const CYCLE: u64 = 1440;

    pub fn new(frames: u64, warmup: u64) -> Self {
        Self {
            warmup,
            frames: frames.max(8),
            frame: 0,
            rss_start: None,
            rss_end: None,
        }
    }

    /// Paints this frame's layers. Returns true after the last frame.
    pub fn frame(&mut self, ctx: &egui::Context) -> bool {
        let last = self.warmup.saturating_add(self.frames);
        if self.frame >= last {
            return true;
        }
        self.frame += 1;
        let n = self.frame;
        let layers = (n % 6) as usize;
        egui::Area::new(Id::new("canopy-glass-soak"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(0.0, 0.0))
            .interactable(false)
            .show(ctx, |ui| {
                let p = theme::palette(ui.ctx());
                let s = theme::surfaces(ui.ctx());
                let style = theme::style();
                for i in 0..layers {
                    let k = n + i as u64 * 97;
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "below 720 after the modulo, exact in f32"
                    )]
                    let size = vec2(80.0 + (k * 37 % 720) as f32, 60.0 + (k * 53 % 480) as f32);
                    let at = pos2(
                        20.0 + crate::count_f32(i) * 40.0,
                        20.0 + crate::count_f32(i) * 30.0,
                    );
                    let rect = Rect::from_min_size(at, size);
                    let radius = CornerRadius::same(theme::metrics().radius_lg);
                    let fill = layer_fill(ui.painter(), rect, radius);
                    super::paint::surface(
                        ui.painter(),
                        rect,
                        radius,
                        p,
                        style,
                        Elevation::Floating,
                        Fill::flat(fill),
                        Some(p.floating_ring(s)),
                    );
                }
            });
        if n == self.warmup.max(1) {
            self.rss_start = resident_kb();
        }
        if n >= last {
            self.rss_end = resident_kb();
            return true;
        }
        ctx.request_repaint();
        false
    }

    /// One line for the soak's caller: measured frames, warm-up, object counters, resident
    /// memory and the blur's state.
    pub fn report(&self) -> String {
        let (created, deleted) = counters();
        let kb = |v: Option<u64>| v.map_or_else(|| "-".to_owned(), |v| v.to_string());
        format!(
            "glass-soak: frames={} warmup={} created={created} deleted={deleted} rss_start_kb={} rss_end_kb={} blur={}",
            self.frame.saturating_sub(self.warmup),
            self.warmup,
            kb(self.rss_start),
            kb(self.rss_end),
            describe()
        )
    }
}

/// This process's resident memory in kilobytes (Linux; elsewhere `None`).
fn resident_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| parse_vm_rss(&s))
}

/// The `VmRSS:  12345 kB` line of `/proc/self/status`.
fn parse_vm_rss(status: &str) -> Option<u64> {
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.trim_start_matches("VmRSS:")
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

// ----------------------------------------------------------------------------------- gallery

/// The gallery section: the blur's state, then the same floating card opaque and translucent
/// over a busy backdrop. The translucent one is forced on whatever the setting says (the theme
/// and the graphics driver still decide).
pub(crate) fn gallery(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let p = theme::palette(&ctx);
    let s = theme::surfaces(&ctx);
    let m = theme::metrics();
    let system = match reduced_transparency() {
        Some(true) => "asks for reduced transparency",
        Some(false) => "allows transparency",
        None => "did not say",
    };
    ui.label(
        RichText::new(format!(
            "Translucent surfaces {} · blur {} · system {} · glass {} at clamp {}",
            if theme::appearance().glass {
                "on"
            } else {
                "off"
            },
            describe(),
            system,
            p.glass_alpha,
            p.glass_luminance
        ))
        .size(m.small_size)
        .color(p.text_dim),
    );
    ui.add_space(theme::SM);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 176.0), Sense::hover());
    busy_backdrop(ui.painter(), rect, p);
    let card_w = ((rect.width() - 3.0 * theme::MD) / 2.0).max(0.0);
    let radius = CornerRadius::same(m.radius_lg);
    for (i, forced) in [false, true].into_iter().enumerate() {
        let card = Rect::from_min_size(
            rect.min + vec2(theme::MD + crate::count_f32(i) * (card_w + theme::MD), 34.0),
            vec2(card_w, 112.0),
        );
        let fill = layer_fill_with(ui.painter(), card, radius, forced);
        let glass = fill.a() < 255;
        super::paint::surface(
            ui.painter(),
            card,
            radius,
            p,
            theme::style(),
            Elevation::Floating,
            Fill::flat(fill),
            Some(p.floating_ring(s)),
        );
        let title = match (forced, glass) {
            (false, _) => "Opaque (off, or the fallback)".to_owned(),
            (true, true) => "Translucent".to_owned(),
            (true, false) if p.glass_alpha == 0 => "Opaque: this theme never uses glass".to_owned(),
            (true, false) => format!("Opaque: blur {}", describe()),
        };
        let at = card.left_top() + vec2(theme::MD, theme::MD);
        let lines = [
            (title.as_str(), p.text, m.body_size),
            ("Secondary text on the layer", p.text_dim, m.small_size),
            ("Faint text, the weakest grade", p.text_faint, m.small_size),
            ("Accent link", p.accent, m.small_size),
            ("Danger text", p.danger, m.small_size),
        ];
        let mut y = at.y;
        for (text, color, size) in lines {
            ui.painter().text(
                pos2(at.x, y),
                Align2::LEFT_TOP,
                text,
                FontId::proportional(size),
                color,
            );
            y += size + theme::XS + 2.0;
        }
    }
}

/// Stripes in the lane colours, light and dark bars and lines of text: content whose blur is easy
/// to see.
fn busy_backdrop(painter: &Painter, rect: Rect, p: &Palette) {
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let bar = 14.0;
    let mut x = rect.left();
    let mut i = 0_u16;
    #[expect(
        clippy::while_float,
        reason = "each step moves a fixed bar width across the rectangle"
    )]
    while x < rect.right() {
        let r = Rect::from_min_max(
            pos2(x, rect.top()),
            pos2((x + bar).min(rect.right()), rect.bottom()),
        );
        painter.rect_filled(r, CornerRadius::ZERO, p.lane(i));
        x += bar;
        i += 1;
    }
    for (k, y) in [0.25_f32, 0.55, 0.85].into_iter().enumerate() {
        let band = Rect::from_center_size(
            pos2(rect.center().x, rect.top() + rect.height() * y),
            vec2(rect.width(), 10.0),
        );
        let c = if k % 2 == 0 {
            Color32::WHITE
        } else {
            Color32::BLACK
        };
        painter.rect_filled(band, CornerRadius::ZERO, c);
    }
    let m = theme::metrics();
    for row in 0..6 {
        painter.text(
            pos2(
                rect.left() + 6.0,
                rect.top() + 6.0 + crate::int_f32(row) * 28.0,
            ),
            Align2::LEFT_TOP,
            "feat(graph): circles in the history graph · fix: keep the selection on refresh",
            FontId::proportional(m.body_size),
            if row % 2 == 0 { p.text } else { Color32::BLACK },
        );
    }
}

#[cfg(test)]
mod tests;

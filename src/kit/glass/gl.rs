#![expect(
    unsafe_code,
    reason = "OpenGL calls for the backdrop blur; owner-approved 2026-10-05"
)]
//! The backdrop blur's OpenGL renderer (design spec 7.4): the only `unsafe` code in Canopy,
//! approved by the owner on the condition that no harm can come from it.
//!
//! Every OpenGL call is one method of [`Current`] holding a single `unsafe` call with its own
//! `SAFETY` comment. What those comments rely on:
//!
//! - **The context is current on this thread.** A [`Current`] is only made from egui_glow's
//!   painter inside its paint callback (egui_glow paints with its context current and hands the
//!   callback that painter) and in [`destroy`], from the context eframe passes to `App::on_exit`,
//!   which it keeps current to delete its own objects right after. `Current` has no constructor
//!   outside this module, and the public way in, [`super::destroy`], takes an `ExitContext`,
//!   which names that call site; safe Rust cannot check that the context is current, so the
//!   token states the condition instead of leaving it to a doc comment. `destroy` makes one only for
//!   the context the objects were made in and only on the thread that made them (the thread that
//!   owns the context); called anywhere else it makes no OpenGL call and leaves the objects to
//!   their context.
//! - **No memory is shared with the driver except fixed-size outputs.** State queries write into
//!   values glow sizes for them, and each query names its state through an enum here whose every
//!   variant has exactly that many values ([`Scalar`], [`Quad`]). Textures are allocated without
//!   client data and with no pixel-unpack buffer bound. Shader sources are Rust strings glow
//!   passes with their length. No pointer or size reaches OpenGL from outside this module.
//! - **Objects belong to the context that made them.** They are created here, used only while the
//!   painter's context is the one they were made in (`Arc::ptr_eq` against the context this module
//!   keeps alive), and deleted once. After a lost context the blur turns off as soon as OpenGL
//!   reports an error and makes no further calls. eframe does not ask for a robust context, so
//!   what a driver does with calls made before that report is its own; Canopy makes none it would
//!   not make on a live context.
//! - **Sizes are checked before use.** Regions, viewports and textures come from
//!   `super::plan`, `super::grow` and `super::level_sizes`, which keep them between 1 pixel and
//!   the driver's texture limit (capped at 4096) and inside the window.
//! - **State is restored.** What the passes change is saved first and put back by a drop guard,
//!   also while a panic unwinds; the caller catches panics so none reaches egui_glow, and turns
//!   the blur off.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::ThreadId;

use eframe::egui_glow;
use eframe::glow::{self, HasContext as _};
use egui::Color32;

use super::{
    Blur, CREATED, DELETED, LEVELS, Look, MIN_TEXTURE_SIDE, Plan, PxRect, Reason, shaders,
};

/// The renderer's place in the facade: nothing built yet, built, or off for the session.
#[derive(Default)]
pub(super) enum Slot {
    #[default]
    Empty,
    Ready(Box<GlassGl>),
    Gone,
}

/// What one paint of a glass layer did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Outcome {
    /// The step is done. `built` carries the texture limit when this paint built the renderer.
    Drawn { built: Option<i32> },
    /// Nothing was blurred: a composite covered the layer with the opaque overlay for this frame
    /// (the region could not be blurred, or the blur is already off); a capture did nothing.
    Covered,
    /// The blur failed and must be turned off; `covered` says whether nothing more needs
    /// painting (a composite drew the opaque overlay, or a capture had nothing to cover).
    Failed { reason: Reason, covered: bool },
}

/// One of the two paint callbacks of a glass layer (`job` identifies the layer in this frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Step {
    /// Copies and blurs what is behind the layer into the levels. Painted before the layer's
    /// shadow, so the shadow is not in the copy.
    Capture { job: u64 },
    /// Draws the blurred copy inside the layer, between its shadow and its fill. Captures first
    /// unless the levels hold this job's capture (the capture callback did not run, or another
    /// layer's capture came in between).
    Composite {
        job: u64,
        look: Look,
        fallback: Color32,
    },
}

impl Step {
    /// Covers a composite's layer with the opaque overlay; a capture has nothing to cover.
    fn cover(self, cur: Current<'_>, plan: &Plan) {
        if let Self::Composite { fallback, .. } = self {
            cover(cur, plan, fallback);
        }
    }

    fn covers(self) -> bool {
        matches!(self, Self::Composite { .. })
    }
}

/// The context egui_glow (or eframe at exit) has made current on this thread. Only made in
/// [`paint`] and [`destroy`]; see the module documentation.
#[derive(Clone, Copy)]
struct Current<'a>(&'a glow::Context);

/// Single-valued integer state, the only kind [`Current::scalar`] may query.
#[derive(Debug, Clone, Copy)]
enum Scalar {
    MaxTextureSize,
    SampleBuffers,
    ActiveTexture,
    BlendSrcRgb,
    BlendDstRgb,
    BlendSrcAlpha,
    BlendDstAlpha,
}

impl Scalar {
    fn name(self) -> u32 {
        match self {
            Self::MaxTextureSize => glow::MAX_TEXTURE_SIZE,
            Self::SampleBuffers => glow::SAMPLE_BUFFERS,
            Self::ActiveTexture => glow::ACTIVE_TEXTURE,
            Self::BlendSrcRgb => glow::BLEND_SRC_RGB,
            Self::BlendDstRgb => glow::BLEND_DST_RGB,
            Self::BlendSrcAlpha => glow::BLEND_SRC_ALPHA,
            Self::BlendDstAlpha => glow::BLEND_DST_ALPHA,
        }
    }
}

/// Four-valued integer state, the only kind [`Current::quad`] may query.
#[derive(Debug, Clone, Copy)]
enum Quad {
    Viewport,
    ScissorBox,
}

impl Quad {
    fn name(self) -> u32 {
        match self {
            Self::Viewport => glow::VIEWPORT,
            Self::ScissorBox => glow::SCISSOR_BOX,
        }
    }
}

/// Capabilities the passes switch.
#[derive(Debug, Clone, Copy)]
enum Cap {
    Scissor,
    Blend,
}

impl Cap {
    fn name(self) -> u32 {
        match self {
            Self::Scissor => glow::SCISSOR_TEST,
            Self::Blend => glow::BLEND,
        }
    }
}

/// Framebuffer binding points.
#[derive(Debug, Clone, Copy)]
enum Target {
    Draw,
    Read,
}

impl Target {
    fn name(self) -> u32 {
        match self {
            Self::Draw => glow::DRAW_FRAMEBUFFER,
            Self::Read => glow::READ_FRAMEBUFFER,
        }
    }

    fn binding(self) -> u32 {
        match self {
            Self::Draw => glow::DRAW_FRAMEBUFFER_BINDING,
            Self::Read => glow::READ_FRAMEBUFFER_BINDING,
        }
    }
}

fn created() {
    CREATED.fetch_add(1, Ordering::Relaxed);
}

fn deleted() {
    DELETED.fetch_add(1, Ordering::Relaxed);
}

impl Current<'_> {
    // ----------------------------------------------------------------------------- queries

    fn scalar(self, which: Scalar) -> i32 {
        // SAFETY: the context is current (`Current`). Every `Scalar` names single-valued state, so
        // glGetIntegerv writes exactly the one i32 glow passes it.
        unsafe { self.0.get_parameter_i32(which.name()) }
    }

    fn quad(self, which: Quad) -> [i32; 4] {
        let mut out = [0; 4];
        // SAFETY: the context is current. VIEWPORT and SCISSOR_BOX hold exactly four integers and
        // `out` has room for four.
        unsafe { self.0.get_parameter_i32_slice(which.name(), &mut out) };
        out
    }

    fn clear_value(self) -> [f32; 4] {
        let mut out = [0.0; 4];
        // SAFETY: the context is current. COLOR_CLEAR_VALUE holds exactly four floats and `out`
        // has room for four.
        unsafe {
            self.0
                .get_parameter_f32_slice(glow::COLOR_CLEAR_VALUE, &mut out);
        };
        out
    }

    fn framebuffer_binding(self, target: Target) -> Option<glow::Framebuffer> {
        // SAFETY: the context is current; a framebuffer binding is one integer.
        unsafe { self.0.get_parameter_framebuffer(target.binding()) }
    }

    fn texture_binding(self) -> Option<glow::Texture> {
        // SAFETY: the context is current; the 2D texture binding of the active unit is one integer.
        unsafe { self.0.get_parameter_texture(glow::TEXTURE_BINDING_2D) }
    }

    fn vertex_array_binding(self) -> Option<glow::VertexArray> {
        // SAFETY: the context is current; the vertex array binding is one integer.
        unsafe {
            self.0
                .get_parameter_vertex_array(glow::VERTEX_ARRAY_BINDING)
        }
    }

    fn program_binding(self) -> Option<glow::Program> {
        // SAFETY: the context is current; the current program is one integer.
        unsafe { self.0.get_parameter_program(glow::CURRENT_PROGRAM) }
    }

    fn unpack_buffer_binding(self) -> Option<glow::Buffer> {
        // SAFETY: the context is current; the pixel-unpack buffer binding is one integer.
        unsafe {
            self.0
                .get_parameter_buffer(glow::PIXEL_UNPACK_BUFFER_BINDING)
        }
    }

    fn is_enabled(self, cap: Cap) -> bool {
        // SAFETY: the context is current; `cap` is a valid capability.
        unsafe { self.0.is_enabled(cap.name()) }
    }

    fn error(self) -> u32 {
        // SAFETY: the context is current; glGetError takes no arguments.
        unsafe { self.0.get_error() }
    }

    // ---------------------------------------------------------------------------- state

    fn set_enabled(self, cap: Cap, on: bool) {
        if on {
            // SAFETY: the context is current; `cap` is a valid capability.
            unsafe { self.0.enable(cap.name()) }
        } else {
            // SAFETY: the context is current; `cap` is a valid capability.
            unsafe { self.0.disable(cap.name()) }
        }
    }

    fn bind_framebuffer(self, target: Target, fbo: Option<glow::Framebuffer>) {
        // SAFETY: the context is current; `fbo` is the default framebuffer, one this module made
        // in this context and has not deleted, or a binding read back from this context.
        unsafe { self.0.bind_framebuffer(target.name(), fbo) }
    }

    fn bind_both(self, fbo: Option<glow::Framebuffer>) {
        // SAFETY: as `bind_framebuffer`; FRAMEBUFFER sets the draw and read bindings together.
        unsafe { self.0.bind_framebuffer(glow::FRAMEBUFFER, fbo) }
    }

    fn active_texture(self, unit: u32) {
        // SAFETY: the context is current; `unit` is TEXTURE0 or a unit read back from it.
        unsafe { self.0.active_texture(unit) }
    }

    fn bind_texture(self, texture: Option<glow::Texture>) {
        // SAFETY: the context is current; `texture` is none, one this module made in this context
        // and has not deleted, or a binding read back from this context.
        unsafe { self.0.bind_texture(glow::TEXTURE_2D, texture) }
    }

    fn bind_vertex_array(self, vao: Option<glow::VertexArray>) {
        // SAFETY: the context is current; `vao` is none, ours, or a binding read back from this
        // context.
        unsafe { self.0.bind_vertex_array(vao) }
    }

    fn use_program(self, program: Option<glow::Program>) {
        // SAFETY: the context is current; `program` is none, a linked program of ours, or the
        // binding read back from this context.
        unsafe { self.0.use_program(program) }
    }

    fn bind_unpack_buffer(self, buffer: Option<glow::Buffer>) {
        // SAFETY: the context is current; `buffer` is none or the binding read back from this
        // context.
        unsafe { self.0.bind_buffer(glow::PIXEL_UNPACK_BUFFER, buffer) }
    }

    fn viewport(self, [x, y, w, h]: [i32; 4]) {
        // SAFETY: the context is current; the values are a validated region or the viewport read
        // back from this context (OpenGL rejects negative sizes with an error, never UB).
        unsafe { self.0.viewport(x, y, w, h) }
    }

    fn scissor(self, [x, y, w, h]: [i32; 4]) {
        // SAFETY: the context is current; as `viewport`.
        unsafe { self.0.scissor(x, y, w, h) }
    }

    fn blend_func(self, [src_rgb, dst_rgb, src_alpha, dst_alpha]: [u32; 4]) {
        // SAFETY: the context is current; the factors are blend factor enums.
        unsafe {
            self.0
                .blend_func_separate(src_rgb, dst_rgb, src_alpha, dst_alpha);
        }
    }

    fn clear_color(self, [r, g, b, a]: [f32; 4]) {
        // SAFETY: the context is current; plain values.
        unsafe { self.0.clear_color(r, g, b, a) }
    }

    fn clear(self) {
        // SAFETY: the context is current; clears the colour of the bound draw framebuffer inside
        // the scissor box set just before.
        unsafe { self.0.clear(glow::COLOR_BUFFER_BIT) }
    }

    // ---------------------------------------------------------------------------- drawing

    /// Copies `src` of the read framebuffer into the bottom-left `dst` pixels of the draw
    /// framebuffer, filtered linearly.
    fn blit(self, src: PxRect, dst: [i32; 2]) {
        let (x1, y1) = (src.x.saturating_add(src.w), src.y.saturating_add(src.h));
        // SAFETY: the context is current; both framebuffers are bound by this module; `src` lies
        // inside the window (`plan`) and `dst` inside our level-0 texture (`level_sizes`,
        // `grow`). No memory is passed.
        unsafe {
            self.0.blit_framebuffer(
                src.x,
                src.y,
                x1,
                y1,
                0,
                0,
                dst[0],
                dst[1],
                glow::COLOR_BUFFER_BIT,
                glow::LINEAR,
            );
        }
    }

    /// Draws the full-viewport triangle (the vertex shader makes its corners).
    fn draw_triangle(self) {
        // SAFETY: the context is current; our vertex array is bound and has no attributes, the
        // vertex shader reads only gl_VertexID, so no buffer is read.
        unsafe { self.0.draw_arrays(glow::TRIANGLES, 0, 3) }
    }

    fn uniform_2(self, at: glow::UniformLocation, [x, y]: [f32; 2]) {
        // SAFETY: the context is current; `at` belongs to the program in use.
        unsafe { self.0.uniform_2_f32(Some(&at), x, y) }
    }

    fn uniform_1(self, at: glow::UniformLocation, v: f32) {
        // SAFETY: the context is current; `at` belongs to the program in use.
        unsafe { self.0.uniform_1_f32(Some(&at), v) }
    }

    fn uniform_4(self, at: glow::UniformLocation, [x, y, z, w]: [f32; 4]) {
        // SAFETY: the context is current; `at` belongs to the program in use.
        unsafe { self.0.uniform_4_f32(Some(&at), x, y, z, w) }
    }

    // --------------------------------------------------------------------------- objects

    #[expect(
        clippy::map_err_ignore,
        reason = "a blur failure is reported as a fixed Reason code, not the driver's text"
    )]
    fn create_shader(self, kind: u32) -> Result<glow::Shader, Reason> {
        // SAFETY: the context is current; `kind` is VERTEX_SHADER or FRAGMENT_SHADER.
        let shader = unsafe { self.0.create_shader(kind) }.map_err(|_| Reason::Compile)?;
        created();
        Ok(shader)
    }

    fn shader_source(self, shader: glow::Shader, source: &str) {
        // SAFETY: the context is current; `shader` is ours; glow passes the string's pointer with
        // its length.
        unsafe { self.0.shader_source(shader, source) }
    }

    fn compile_shader(self, shader: glow::Shader) {
        // SAFETY: the context is current; `shader` is ours.
        unsafe { self.0.compile_shader(shader) }
    }

    fn compiled(self, shader: glow::Shader) -> bool {
        // SAFETY: the context is current; `shader` is ours; the status is one integer.
        unsafe { self.0.get_shader_compile_status(shader) }
    }

    fn delete_shader(self, shader: glow::Shader) {
        // SAFETY: the context is current; `shader` is ours and deleted only here, once.
        unsafe { self.0.delete_shader(shader) }
        deleted();
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "a blur failure is reported as a fixed Reason code, not the driver's text"
    )]
    fn create_program(self) -> Result<glow::Program, Reason> {
        // SAFETY: the context is current.
        let program = unsafe { self.0.create_program() }.map_err(|_| Reason::Link)?;
        created();
        Ok(program)
    }

    fn attach_shader(self, program: glow::Program, shader: glow::Shader) {
        // SAFETY: the context is current; both objects are ours.
        unsafe { self.0.attach_shader(program, shader) }
    }

    fn detach_shader(self, program: glow::Program, shader: glow::Shader) {
        // SAFETY: the context is current; both objects are ours and `shader` is attached.
        unsafe { self.0.detach_shader(program, shader) }
    }

    fn bind_output(self, program: glow::Program) {
        // SAFETY: the context is current (desktop OpenGL 3.0 or later); `program` is ours and
        // not linked yet; the name is a constant without NUL bytes.
        unsafe { self.0.bind_frag_data_location(program, 0, "o_color") }
    }

    fn link_program(self, program: glow::Program) {
        // SAFETY: the context is current; `program` is ours.
        unsafe { self.0.link_program(program) }
    }

    fn linked(self, program: glow::Program) -> bool {
        // SAFETY: the context is current; `program` is ours; the status is one integer.
        unsafe { self.0.get_program_link_status(program) }
    }

    fn delete_program(self, program: glow::Program) {
        // SAFETY: the context is current; `program` is ours and deleted only here, once.
        unsafe { self.0.delete_program(program) }
        deleted();
    }

    fn uniform_location(self, program: glow::Program, name: &str) -> Option<glow::UniformLocation> {
        // SAFETY: the context is current; `program` is ours and linked; `name` is a constant
        // without NUL bytes.
        unsafe { self.0.get_uniform_location(program, name) }
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "a blur failure is reported as a fixed Reason code, not the driver's text"
    )]
    fn create_vertex_array(self) -> Result<glow::VertexArray, Reason> {
        // SAFETY: the context is current (OpenGL 3.0 or ES 3.0, where vertex arrays are core).
        let vao = unsafe { self.0.create_vertex_array() }.map_err(|_| Reason::Internal)?;
        created();
        Ok(vao)
    }

    fn delete_vertex_array(self, vao: glow::VertexArray) {
        // SAFETY: the context is current; `vao` is ours and deleted only here, once.
        unsafe { self.0.delete_vertex_array(vao) }
        deleted();
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "a blur failure is reported as a fixed Reason code, not the driver's text"
    )]
    fn create_texture(self) -> Result<glow::Texture, Reason> {
        // SAFETY: the context is current.
        let texture = unsafe { self.0.create_texture() }.map_err(|_| Reason::Framebuffer)?;
        created();
        Ok(texture)
    }

    fn delete_texture(self, texture: glow::Texture) {
        // SAFETY: the context is current; `texture` is ours and deleted only here, once.
        unsafe { self.0.delete_texture(texture) }
        deleted();
    }

    fn texture_parameter(self, name: u32, value: u32) {
        let value = i32::try_from(value).unwrap_or(0);
        // SAFETY: the context is current; one of our textures is bound; `name` and `value` are a
        // filter or wrap parameter and its enum.
        unsafe { self.0.tex_parameter_i32(glow::TEXTURE_2D, name, value) }
    }

    /// Gives the bound texture `size` pixels of RGBA8 storage without uploading anything.
    fn allocate(self, size: [i32; 2]) {
        let format = i32::try_from(glow::RGBA8).unwrap_or(0);
        // SAFETY: the context is current; one of our textures is bound; `size` is between 1 and
        // the texture limit (`grow`, `level_sizes`); no client data is passed and no
        // pixel-unpack buffer is bound (`GlassGl::ensure`), so OpenGL reads no memory.
        unsafe {
            self.0.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                format,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
        }
    }

    #[expect(
        clippy::map_err_ignore,
        reason = "a blur failure is reported as a fixed Reason code, not the driver's text"
    )]
    fn create_framebuffer(self) -> Result<glow::Framebuffer, Reason> {
        // SAFETY: the context is current.
        let fbo = unsafe { self.0.create_framebuffer() }.map_err(|_| Reason::Framebuffer)?;
        created();
        Ok(fbo)
    }

    fn delete_framebuffer(self, fbo: glow::Framebuffer) {
        // SAFETY: the context is current; `fbo` is ours and deleted only here, once.
        unsafe { self.0.delete_framebuffer(fbo) }
        deleted();
    }

    fn attach_texture(self, texture: glow::Texture) {
        // SAFETY: the context is current; one of our framebuffers is bound to FRAMEBUFFER and
        // `texture` is ours.
        unsafe {
            self.0.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
        }
    }

    fn framebuffer_complete(self) -> bool {
        // SAFETY: the context is current; one of our framebuffers is bound to FRAMEBUFFER.
        unsafe { self.0.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE }
    }

    // ----------------------------------------------------------------------------- errors

    /// Clears errors raised before the blur ran (not ours); a lost context is reported.
    fn drain_errors(self) -> Result<(), Reason> {
        for _ in 0..8 {
            match self.error() {
                glow::NO_ERROR => return Ok(()),
                glow::CONTEXT_LOST => return Err(Reason::ContextLost),
                code => tracing::debug!("OpenGL error 0x{code:04X} pending before the blur"),
            }
        }
        Ok(())
    }

    /// The first error the blur's own calls raised, if any.
    fn check_errors(self) -> Result<(), Reason> {
        let first = self.error();
        if first == glow::NO_ERROR {
            return Ok(());
        }
        // Read the rest so egui does not see them; bounded, a lost context may keep answering.
        for _ in 0..8 {
            if self.error() == glow::NO_ERROR {
                break;
            }
        }
        Err(if first == glow::CONTEXT_LOST {
            Reason::ContextLost
        } else {
            Reason::Gl(first)
        })
    }
}

/// Everything the passes change, read back before they run and put back by [`Guard`].
struct Saved {
    draw: Option<glow::Framebuffer>,
    read: Option<glow::Framebuffer>,
    active_texture: u32,
    texture: Option<glow::Texture>,
    vao: Option<glow::VertexArray>,
    program: Option<glow::Program>,
    viewport: [i32; 4],
    scissor_box: [i32; 4],
    scissor: bool,
    blend: bool,
    blend_func: [u32; 4],
}

impl Saved {
    fn capture(cur: Current<'_>) -> Self {
        let factor = |which: Scalar| u32::try_from(cur.scalar(which)).unwrap_or(glow::ONE);
        let active_texture =
            u32::try_from(cur.scalar(Scalar::ActiveTexture)).unwrap_or(glow::TEXTURE0);
        cur.active_texture(glow::TEXTURE0);
        Self {
            draw: cur.framebuffer_binding(Target::Draw),
            read: cur.framebuffer_binding(Target::Read),
            active_texture,
            texture: cur.texture_binding(),
            vao: cur.vertex_array_binding(),
            program: cur.program_binding(),
            viewport: cur.quad(Quad::Viewport),
            scissor_box: cur.quad(Quad::ScissorBox),
            scissor: cur.is_enabled(Cap::Scissor),
            blend: cur.is_enabled(Cap::Blend),
            blend_func: [
                factor(Scalar::BlendSrcRgb),
                factor(Scalar::BlendDstRgb),
                factor(Scalar::BlendSrcAlpha),
                factor(Scalar::BlendDstAlpha),
            ],
        }
    }

    fn restore(&self, cur: Current<'_>) {
        cur.bind_framebuffer(Target::Read, self.read);
        cur.bind_framebuffer(Target::Draw, self.draw);
        cur.active_texture(glow::TEXTURE0);
        cur.bind_texture(self.texture);
        cur.active_texture(self.active_texture);
        cur.bind_vertex_array(self.vao);
        cur.use_program(self.program);
        cur.viewport(self.viewport);
        cur.scissor(self.scissor_box);
        cur.set_enabled(Cap::Scissor, self.scissor);
        cur.set_enabled(Cap::Blend, self.blend);
        cur.blend_func(self.blend_func);
    }
}

/// Restores [`Saved`] state when dropped, also while a panic unwinds.
struct Guard<'a> {
    cur: Current<'a>,
    saved: Saved,
}

impl<'a> Guard<'a> {
    fn new(cur: Current<'a>) -> Self {
        Self {
            cur,
            saved: Saved::capture(cur),
        }
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.saved.restore(self.cur);
    }
}

/// A blur pass program and its uniforms.
struct BlurProgram {
    program: glow::Program,
    texel: glow::UniformLocation,
    src_size: glow::UniformLocation,
    dst_size: glow::UniformLocation,
    offset: glow::UniformLocation,
}

/// The composite program and its uniforms.
struct CompositeProgram {
    program: glow::Program,
    texel: glow::UniformLocation,
    src_size: glow::UniformLocation,
    region: glow::UniformLocation,
    rect: glow::UniformLocation,
    radii: glow::UniformLocation,
    look: glow::UniformLocation,
}

/// One blur level: a texture, the framebuffer that draws into it, and its allocated size.
struct Level {
    texture: glow::Texture,
    fbo: glow::Framebuffer,
    size: [i32; 2],
}

/// Objects made while building, deleted if building stops half way (also on a panic).
struct Building<'a> {
    cur: Current<'a>,
    programs: Vec<glow::Program>,
    vao: Option<glow::VertexArray>,
    levels: Vec<Level>,
}

impl Drop for Building<'_> {
    fn drop(&mut self) {
        for program in self.programs.drain(..) {
            self.cur.delete_program(program);
        }
        if let Some(vao) = self.vao.take() {
            self.cur.delete_vertex_array(vao);
        }
        for level in self.levels.drain(..) {
            delete_level(self.cur, level);
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "taking the level means it cannot be used once its objects are deleted"
)]
fn delete_level(cur: Current<'_>, level: Level) {
    cur.delete_framebuffer(level.fbo);
    cur.delete_texture(level.texture);
}

/// One texel of a texture of `size` pixels, in texture coordinates.
fn texel(size: [i32; 2]) -> [f32; 2] {
    [1.0 / crate::int_f32(size[0]), 1.0 / crate::int_f32(size[1])]
}

/// A size in pixels as shader floats.
fn size(s: [i32; 2]) -> [f32; 2] {
    [crate::int_f32(s[0]), crate::int_f32(s[1])]
}

/// The renderer: programs, an empty vertex array and the blur levels, all made in `gl`.
pub(super) struct GlassGl {
    /// The context the objects belong to, kept alive so its address identifies it.
    gl: Arc<glow::Context>,
    /// The thread that made the objects: the one on which their context is current.
    thread: ThreadId,
    max_side: i32,
    down: BlurProgram,
    up: BlurProgram,
    composite: CompositeProgram,
    vao: glow::VertexArray,
    levels: Option<[Level; LEVELS]>,
    /// The region size the levels were allocated for.
    capacity: [i32; 2],
    /// The job and window region whose blurred copy level 0 holds, until its composite uses it.
    captured: Option<(u64, PxRect)>,
    /// False once the objects are deleted or left to a context that went away.
    live: bool,
}

impl Drop for GlassGl {
    fn drop(&mut self) {
        if self.live {
            tracing::warn!("translucent surfaces: blur objects left to their OpenGL context");
        }
    }
}

/// Compiles and links one program from the shared vertex shader and `fragment`.
fn program(
    cur: Current<'_>,
    header: &str,
    fragment: &str,
    es: bool,
) -> Result<glow::Program, Reason> {
    let vs = cur.create_shader(glow::VERTEX_SHADER)?;
    let fs = match cur.create_shader(glow::FRAGMENT_SHADER) {
        Ok(fs) => fs,
        Err(e) => {
            cur.delete_shader(vs);
            return Err(e);
        }
    };
    let shaders = [
        (vs, format!("{header}{}", shaders::VERTEX)),
        (fs, format!("{header}{fragment}")),
    ];
    let mut ok = true;
    for (shader, source) in &shaders {
        cur.shader_source(*shader, source);
        cur.compile_shader(*shader);
        ok &= cur.compiled(*shader);
    }
    // The driver's info log is not read: glow copies it into a String without checking it is
    // UTF-8, and the log would say nothing a user can act on.
    let result = if ok {
        match cur.create_program() {
            Ok(program) => {
                cur.attach_shader(program, vs);
                cur.attach_shader(program, fs);
                if !es {
                    cur.bind_output(program);
                }
                cur.link_program(program);
                let linked = cur.linked(program);
                cur.detach_shader(program, vs);
                cur.detach_shader(program, fs);
                if linked {
                    Ok(program)
                } else {
                    cur.delete_program(program);
                    Err(Reason::Link)
                }
            }
            Err(e) => Err(e),
        }
    } else {
        Err(Reason::Compile)
    };
    cur.delete_shader(vs);
    cur.delete_shader(fs);
    result
}

fn location(
    cur: Current<'_>,
    program: glow::Program,
    name: &str,
) -> Result<glow::UniformLocation, Reason> {
    cur.uniform_location(program, name).ok_or(Reason::Uniform)
}

fn blur_program(cur: Current<'_>, program: glow::Program) -> Result<BlurProgram, Reason> {
    Ok(BlurProgram {
        program,
        texel: location(cur, program, "u_texel")?,
        src_size: location(cur, program, "u_src_size")?,
        dst_size: location(cur, program, "u_dst_size")?,
        offset: location(cur, program, "u_offset")?,
    })
}

/// Makes one level's texture and framebuffer at `size` (a texture is bound and the framebuffer
/// left bound; the caller's guard restores both).
fn make_level(cur: Current<'_>, size: [i32; 2]) -> Result<Level, Reason> {
    let texture = cur.create_texture()?;
    cur.bind_texture(Some(texture));
    cur.texture_parameter(glow::TEXTURE_MIN_FILTER, glow::LINEAR);
    cur.texture_parameter(glow::TEXTURE_MAG_FILTER, glow::LINEAR);
    cur.texture_parameter(glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE);
    cur.texture_parameter(glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE);
    cur.allocate(size);
    let fbo = match cur.create_framebuffer() {
        Ok(fbo) => fbo,
        Err(e) => {
            cur.delete_texture(texture);
            return Err(e);
        }
    };
    let level = Level { texture, fbo, size };
    cur.bind_both(Some(fbo));
    cur.attach_texture(texture);
    if cur.framebuffer_complete() {
        Ok(level)
    } else {
        delete_level(cur, level);
        Err(Reason::Framebuffer)
    }
}

impl GlassGl {
    /// Builds the programs and the vertex array (the levels come with the first region).
    fn new(cur: Current<'_>, gl: &Arc<glow::Context>) -> Result<Self, Reason> {
        let version = gl.version();
        let es = version.is_embedded;
        let header = shaders::header(version.major, version.minor, es).ok_or(Reason::OldGl)?;
        let max_side = cur.scalar(Scalar::MaxTextureSize).min(super::MAX_SIDE_CAP);
        if max_side < MIN_TEXTURE_SIDE {
            return Err(Reason::SmallTextures);
        }
        // A multisampled window cannot be read by a scaled blit.
        if cur.scalar(Scalar::SampleBuffers) > 0 {
            return Err(Reason::Multisampled);
        }
        let mut building = Building {
            cur,
            programs: Vec::new(),
            vao: None,
            levels: Vec::new(),
        };
        let (down_src, up_src) = (shaders::down(), shaders::up());
        for fragment in [down_src.as_str(), up_src.as_str(), shaders::COMPOSITE] {
            let p = program(cur, header, fragment, es)?;
            building.programs.push(p);
        }
        building.vao = Some(cur.create_vertex_array()?);
        let [down, up, composite] = [0, 1, 2].map(|i| building.programs.get(i).copied());
        let (Some(down), Some(up), Some(composite), Some(vao)) =
            (down, up, composite, building.vao)
        else {
            return Err(Reason::Internal);
        };
        // Sampler uniforms start at 0, texture unit 0, which is the unit every pass uses.
        let glass = Self {
            gl: Arc::clone(gl),
            thread: std::thread::current().id(),
            max_side,
            down: blur_program(cur, down)?,
            up: blur_program(cur, up)?,
            composite: CompositeProgram {
                program: composite,
                texel: location(cur, composite, "u_texel")?,
                src_size: location(cur, composite, "u_src_size")?,
                region: location(cur, composite, "u_region")?,
                rect: location(cur, composite, "u_rect")?,
                radii: location(cur, composite, "u_radii")?,
                look: location(cur, composite, "u_look")?,
            },
            vao,
            levels: None,
            capacity: [0, 0],
            captured: None,
            live: true,
        };
        // Built: the objects now belong to `glass`.
        building.programs.clear();
        building.vao = None;
        Ok(glass)
    }

    /// Makes sure the levels hold a region of `needed` pixels, growing them if not.
    fn ensure(&mut self, cur: Current<'_>, needed: [i32; 2]) -> Result<(), Reason> {
        let capacity = super::grow(needed, self.capacity, self.max_side).ok_or(Reason::Internal)?;
        if self.levels.is_some() && capacity == self.capacity {
            return Ok(());
        }
        if let Some(levels) = self.levels.take() {
            for level in levels {
                delete_level(cur, level);
            }
        }
        self.capacity = [0, 0];
        self.captured = None;
        // A bound pixel-unpack buffer would make the allocation read from it.
        let unpack = cur.unpack_buffer_binding();
        if unpack.is_some() {
            cur.bind_unpack_buffer(None);
        }
        let mut building = Building {
            cur,
            programs: Vec::new(),
            vao: None,
            levels: Vec::with_capacity(LEVELS),
        };
        let mut result = Ok(());
        for size in super::level_sizes(capacity) {
            match make_level(cur, size) {
                Ok(level) => building.levels.push(level),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        if unpack.is_some() {
            cur.bind_unpack_buffer(unpack);
        }
        result?;
        let levels: Vec<Level> = std::mem::take(&mut building.levels);
        let levels: [Level; LEVELS] = levels.try_into().map_err(|rest: Vec<Level>| {
            for level in rest {
                delete_level(cur, level);
            }
            Reason::Internal
        })?;
        self.levels = Some(levels);
        self.capacity = capacity;
        Ok(())
    }

    /// Copies and blurs the region behind a layer into level 0, for the composite of `job`.
    /// `saved` holds egui's framebuffer binding.
    fn capture(
        &mut self,
        cur: Current<'_>,
        saved: &Saved,
        blur: Blur,
        job: u64,
    ) -> Result<(), Reason> {
        self.captured = None;
        self.ensure(cur, [blur.region.w, blur.region.h])?;
        self.blur(cur, saved, blur)?;
        self.captured = Some((job, blur.region));
        Ok(())
    }

    /// Draws `job`'s blurred copy inside its layer, capturing it first unless level 0 holds it.
    fn present(
        &mut self,
        cur: Current<'_>,
        saved: &Saved,
        plan: &Plan,
        blur: Blur,
        look: Look,
        job: u64,
    ) -> Result<(), Reason> {
        if self.captured != Some((job, blur.region)) {
            self.capture(cur, saved, blur, job)?;
        }
        // One composite per capture: the next layer copies what is behind it.
        self.captured = None;
        self.draw(cur, saved, plan, blur, look)
    }

    /// Copies the region at half size into level 0, then blurs it down the pyramid and back.
    fn blur(&self, cur: Current<'_>, saved: &Saved, blur: Blur) -> Result<(), Reason> {
        let Some(levels) = &self.levels else {
            return Err(Reason::Internal);
        };
        let level = |i: usize| levels.get(i).ok_or(Reason::Internal);
        let used = super::level_sizes([blur.region.w, blur.region.h]);
        let used_at = |i: usize| used.get(i).copied().ok_or(Reason::Internal);

        cur.set_enabled(Cap::Scissor, false);
        cur.set_enabled(Cap::Blend, false);
        cur.bind_vertex_array(Some(self.vao));
        cur.active_texture(glow::TEXTURE0);

        // 1. The region behind the layer, at half size.
        cur.bind_framebuffer(Target::Read, saved.draw);
        cur.bind_framebuffer(Target::Draw, Some(level(0)?.fbo));
        cur.blit(blur.region, used_at(0)?);

        // 2. Down the pyramid, then 3. back up to level 0.
        let pass = |p: &BlurProgram, from: usize, to: usize| -> Result<(), Reason> {
            let (src, dst) = (level(from)?, level(to)?);
            let dst_used = used_at(to)?;
            cur.bind_both(Some(dst.fbo));
            cur.viewport([0, 0, dst_used[0], dst_used[1]]);
            cur.bind_texture(Some(src.texture));
            cur.uniform_2(p.texel, texel(src.size));
            cur.uniform_2(p.src_size, size(used_at(from)?));
            cur.uniform_2(p.dst_size, size(dst_used));
            cur.uniform_1(p.offset, blur.offset);
            cur.draw_triangle();
            Ok(())
        };
        let passes = blur.passes.min(LEVELS - 1);
        cur.use_program(Some(self.down.program));
        for k in 0..passes {
            pass(&self.down, k, k + 1)?;
        }
        cur.use_program(Some(self.up.program));
        for k in (0..passes).rev() {
            pass(&self.up, k + 1, k)?;
        }
        Ok(())
    }

    /// 4. Draws level 0 into egui's framebuffer, inside the layer's rounded rectangle.
    fn draw(
        &self,
        cur: Current<'_>,
        saved: &Saved,
        plan: &Plan,
        blur: Blur,
        look: Look,
    ) -> Result<(), Reason> {
        let Some(levels) = &self.levels else {
            return Err(Reason::Internal);
        };
        let level0 = levels.first().ok_or(Reason::Internal)?;
        let used0 = super::level_sizes([blur.region.w, blur.region.h])
            .first()
            .copied()
            .ok_or(Reason::Internal)?;
        let b = plan.bounds;
        let s = plan.scissor;
        let r = blur.region;
        let c = &self.composite;
        cur.bind_both(saved.draw);
        cur.viewport([b.x, b.y, b.w, b.h]);
        cur.set_enabled(Cap::Scissor, true);
        cur.scissor([s.x, s.y, s.w, s.h]);
        cur.set_enabled(Cap::Blend, true);
        cur.blend_func([
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
            glow::ONE_MINUS_DST_ALPHA,
            glow::ONE,
        ]);
        cur.bind_vertex_array(Some(self.vao));
        cur.active_texture(glow::TEXTURE0);
        cur.use_program(Some(c.program));
        cur.bind_texture(Some(level0.texture));
        cur.uniform_2(c.texel, texel(level0.size));
        cur.uniform_2(c.src_size, size(used0));
        cur.uniform_4(
            c.region,
            [
                crate::int_f32(r.x),
                crate::int_f32(r.y),
                crate::int_f32(r.w),
                crate::int_f32(r.h),
            ],
        );
        cur.uniform_4(c.rect, plan.rect);
        cur.uniform_4(c.radii, plan.radii);
        cur.uniform_4(c.look, look.uniform());
        cur.draw_triangle();
        Ok(())
    }

    /// Deletes every object (the context must be the one they were made in).
    fn delete(&mut self, cur: Current<'_>) {
        if !self.live {
            return;
        }
        if let Some(levels) = self.levels.take() {
            for level in levels {
                delete_level(cur, level);
            }
        }
        cur.delete_program(self.down.program);
        cur.delete_program(self.up.program);
        cur.delete_program(self.composite.program);
        cur.delete_vertex_array(self.vao);
        self.captured = None;
        self.live = false;
    }

    /// Forgets the objects without deleting them: their context is gone or not current.
    fn abandon(&mut self) {
        if self.live {
            tracing::debug!("translucent surfaces: blur objects left to a previous context");
        }
        self.levels = None;
        self.captured = None;
        self.live = false;
    }
}

/// Runs one step of a glass layer: builds the renderer on first use, then captures (copies and
/// blurs) or composites. On any failure it deletes what it built, covers a composite's layer with
/// its fallback and reports the reason; the caller turns the blur off.
pub(super) fn paint(
    slot: &mut Slot,
    painter: &egui_glow::Painter,
    plan: &Plan,
    step: Step,
) -> Outcome {
    let gl = painter.gl();
    let cur = Current(gl);
    if let Err(reason) = cur.drain_errors() {
        if let Slot::Ready(glass) = slot {
            glass.abandon();
        }
        *slot = Slot::Gone;
        return Outcome::Failed {
            reason,
            covered: false,
        };
    }
    let Some(blur) = plan.blur else {
        step.cover(cur, plan);
        return Outcome::Covered;
    };
    if let Slot::Ready(glass) = slot
        && !Arc::ptr_eq(&glass.gl, gl)
    {
        // Never happens with eframe on the desktop; the old objects died with their context.
        glass.abandon();
        *slot = Slot::Empty;
    }
    if matches!(slot, Slot::Gone) {
        step.cover(cur, plan);
        return Outcome::Covered;
    }

    let guard = Guard::new(cur);
    let mut built = None;
    if matches!(slot, Slot::Empty) {
        match GlassGl::new(cur, gl) {
            Ok(glass) => {
                built = Some(glass.max_side);
                *slot = Slot::Ready(Box::new(glass));
            }
            Err(reason) => {
                drop(guard);
                *slot = Slot::Gone;
                let _ = cur.check_errors();
                step.cover(cur, plan);
                return Outcome::Failed {
                    reason,
                    covered: true,
                };
            }
        }
    }
    let Slot::Ready(glass) = slot else {
        drop(guard);
        step.cover(cur, plan);
        return Outcome::Covered;
    };
    // A region over the driver's limit, seen before the limit was known: opaque this frame.
    if super::grow([blur.region.w, blur.region.h], [0, 0], glass.max_side).is_none() {
        drop(guard);
        step.cover(cur, plan);
        return Outcome::Covered;
    }
    let drawn = match step {
        Step::Capture { job } => glass.capture(cur, &guard.saved, blur, job),
        Step::Composite { job, look, .. } => {
            glass.present(cur, &guard.saved, plan, blur, look, job)
        }
    };
    drop(guard);
    match drawn.and_then(|()| cur.check_errors()) {
        Ok(()) => Outcome::Drawn { built },
        Err(reason) => {
            if reason == Reason::ContextLost {
                glass.abandon();
            } else {
                glass.delete(cur);
                let _ = cur.check_errors();
            }
            *slot = Slot::Gone;
            let covered = reason != Reason::ContextLost || !step.covers();
            if reason != Reason::ContextLost {
                step.cover(cur, plan);
            }
            Outcome::Failed { reason, covered }
        }
    }
}

/// Covers a layer whose blur failed in this paint with the opaque overlay, so its translucent
/// fill never sits over unclamped content.
pub(super) fn fallback(painter: &egui_glow::Painter, plan: &Plan, color: Color32) {
    cover(Current(painter.gl()), plan, color);
}

/// Clears the layer's two corner-free rectangles (see [`Plan::cross`]) to `color` in the bound
/// draw framebuffer (egui's: every path restores it first), restoring the scissor and the clear
/// colour after.
fn cover(cur: Current<'_>, plan: &Plan, color: Color32) {
    let scissor = cur.is_enabled(Cap::Scissor);
    let scissor_box = cur.quad(Quad::ScissorBox);
    let clear = cur.clear_value();
    let [r, g, b, _] = color.to_array();
    let unit = |v: u8| f32::from(v) / 255.0;
    cur.clear_color([unit(r), unit(g), unit(b), 1.0]);
    cur.set_enabled(Cap::Scissor, true);
    for rect in plan.cross().into_iter().flatten() {
        cur.scissor([rect.x, rect.y, rect.w, rect.h]);
        cur.clear();
    }
    cur.clear_color(clear);
    cur.scissor(scissor_box);
    cur.set_enabled(Cap::Scissor, scissor);
}

/// Deletes the renderer's objects at exit and marks the blur gone. `gl` is the context inside an
/// [`ExitContext`], the one eframe passed to `App::on_exit`, which is why it may be treated as
/// current. Deletes only when it is the context the objects were made in and this is the thread
/// that made them; otherwise it makes no OpenGL call and leaves them to their context (they die
/// with it).
pub(super) fn destroy(slot: &mut Slot, gl: Option<&glow::Context>) {
    if let Slot::Ready(glass) = slot {
        match gl {
            Some(gl)
                if owns(
                    (Arc::as_ptr(&glass.gl), glass.thread),
                    (gl, std::thread::current().id()),
                ) =>
            {
                glass.delete(Current(gl));
            }
            _ => glass.abandon(),
        }
    }
    *slot = Slot::Gone;
}

/// Whether objects made in context `made.0` on thread `made.1` may be deleted from context
/// `here.0` on thread `here.1`: only in their own context, on the thread that made them.
pub(super) fn owns(
    made: (*const glow::Context, ThreadId),
    here: (*const glow::Context, ThreadId),
) -> bool {
    std::ptr::eq(made.0, here.0) && made.1 == here.1
}

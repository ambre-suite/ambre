// render/image.wgsl
// Textured-quad renderer for decoded raster images (`DrawSurface::draw_image`).
//
// One bind group per image texture, so a batch is drawn per distinct picture
// rather than per element: the vertex layout carries no colour and no opacity,
// only the destination corner and the texel it takes. Nothing on a board asks a
// picture to fade yet, and a per-vertex alpha nobody writes is weight in every
// buffer -- it is added the day an element carries an opacity.
//
// The sampler is trilinear over a mip chain built on the CPU. Minifying a
// picture through the top level alone makes each screen pixel take one texel out
// of many, which crawls as the board pans; the chain is what keeps a photograph
// still.
//
// Samples are non-premultiplied RGBA8, matching `ImageSource`, and the pipeline
// blends them with the same ALPHA_BLENDING state as every other pass.

struct Uniforms {
    // Canvas size in logical pixels.
    viewport: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct VertIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv:  vec2<f32>,
}

struct VertOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@vertex
fn vs_main(v: VertIn) -> VertOut {
    var out: VertOut;
    // Pixel (0, 0) -> NDC (-1, +1); pixel (W, H) -> NDC (+1, -1).
    out.clip_pos = vec4<f32>(
        v.pos.x / u.viewport.x * 2.0 - 1.0,
        1.0 - v.pos.y / u.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = v.uv;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    return textureSample(tex, tex_sampler, in.uv);
}

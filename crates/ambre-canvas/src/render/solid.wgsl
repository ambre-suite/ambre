// render/solid.wgsl
// Solid-color quad renderer for Ambre UI Thread.
//
// Vertex positions are in logical pixel space: origin top-left, Y down.
// The viewport uniform converts them to WebGPU NDC at draw time.
// Colors are straight-alpha sRGB f32 [r, g, b, a] -- blending is handled
// by the ALPHA_BLENDING pipeline state (no premultiply needed here).

struct Uniforms {
    // Canvas size in logical pixels.
    viewport: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertIn {
    @location(0) pos:   vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct VertOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       color:    vec4<f32>,
}

@vertex
fn vs_main(v: VertIn) -> VertOut {
    var out: VertOut;
    // Pixel (0, 0)    -> NDC (-1, +1)
    // Pixel (W, H)    -> NDC (+1, -1)
    out.clip_pos = vec4<f32>(
        v.pos.x / u.viewport.x * 2.0 - 1.0,
        1.0 - v.pos.y / u.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    return in.color;
}

// render/blit.wgsl
// Fullscreen blit: samples a texture and writes it to the swapchain.
// Used to copy the persistent cells offscreen onto the final framebuffer.

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

struct VertOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertOut {
    // Two-triangle fullscreen quad generated from vertex index (no VB).
    //   0,2,4 → (-1,-1), (3,-1), (-1,3)
    //  Plus the triangle covers the whole NDC quad with UVs in [0,1].
    var uv = vec2<f32>(
        f32((idx << 1u) & 2u),  // 0,2,0
        f32(idx & 2u),          // 0,0,2
    );
    var out: VertOut;
    out.clip_pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_sampler, in.uv);
}

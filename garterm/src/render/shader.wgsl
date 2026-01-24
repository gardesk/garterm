// Terminal renderer shader

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) is_glyph: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) is_glyph: f32,
}

@group(0) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(0) @binding(1)
var atlas_sampler: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.is_glyph = in.is_glyph;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.is_glyph > 0.5) {
        // Glyph rendering - sample alpha from atlas
        let alpha = textureSample(atlas_texture, atlas_sampler, in.uv).r;
        // Discard fully transparent pixels to avoid blending issues
        if (alpha < 0.01) {
            discard;
        }
        return vec4<f32>(in.color.rgb, alpha);
    } else {
        // Solid color (backgrounds, cursor)
        return in.color;
    }
}

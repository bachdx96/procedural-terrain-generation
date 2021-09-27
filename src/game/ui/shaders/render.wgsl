struct UVColor {
  color: vec4<f32>;
  uv: vec2<f32>;
};

[[block]]
struct Transform {
  scale: vec2<f32>;
  translate: vec2<f32>;
};

struct VertexOutput {
  [[location(0)]] uv : vec2<f32>;
  [[location(1)]] color : vec4<f32>;
  [[builtin(position)]] position : vec4<f32>;
};

[[group(1), binding(0)]] var<uniform> transform : Transform;

[[stage(vertex)]]
fn vs_main(
  [[location(0)]] pos : vec2<f32>,
  [[location(1)]] uv : vec2<f32>,
  [[location(2)]] color : vec4<f32>) -> VertexOutput {
  var out: VertexOutput;
  out.uv = uv;
  out.color = color;

  var scale : vec2<f32> = vec2<f32>(transform.scale.x, transform.scale.y*-1.0);
  var translate : vec2<f32> = vec2<f32>(transform.translate.x, transform.translate.y+2.0);
  out.position = vec4<f32>(pos * scale + translate, 0.0, 1.0);
  return out;
}

// fragment
[[group(0), binding(1)]] var u_texture : texture_2d<f32>;
[[group(0), binding(0)]] var u_sampler : sampler;

[[stage(fragment)]]
fn fs_main([[location(0)]] uv : vec2<f32>, [[location(1)]] color : vec4<f32>) -> [[location(0)]] vec4<f32> {
  return color * textureSample(u_texture, u_sampler, uv);
}
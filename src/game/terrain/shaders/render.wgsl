struct VertexOutput {
    [[builtin(position)]] position: vec4<f32>;
    [[location(0)]] color: vec4<f32>;
    [[location(1)]] normal: vec4<f32>;
};

[[block]]
struct MeshData {
    world_matrix: mat4x4<f32>;
};

[[group(0), binding(0)]]
var mesh_data: MeshData;

[[block]]
struct CameraData {
    view_matrix: mat4x4<f32>;
    projection_matrix: mat4x4<f32>;
};

[[group(0), binding(1)]]
var camera_data: CameraData;

[[stage(vertex)]]
fn main(
    [[location(0)]] position: vec4<f32>,
    [[location(1)]] normal: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    var p =
        camera_data.projection_matrix * 
        camera_data.view_matrix * 
        mesh_data.world_matrix * 
        position;
    out.color = vec4<f32>(0.0, 0.8, 0.5, 1.0);
    out.position = p;
    out.normal = normal;
    return out;
}

[[stage(fragment)]]
fn main([[location(0)]] color : vec4<f32>, [[location(1)]] normal : vec4<f32>) -> [[location(0)]] vec4<f32> {
    let normal = normalize(normal.xyz);
    let light_dir = vec3<f32>(0.0,0.0,-1.0);
    return vec4<f32>(normal.xyz / 2.0 + 0.5, 1.0);
}
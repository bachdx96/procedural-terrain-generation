// GLOBALS
let SHADER_WORKGROUP_SIZE: u32 = 8u;

// STRUCTS

[[block]]
struct GenerateVoxelInfo {
    voxel_count: vec3<u32>;
    lod: u32;
    min: vec3<f32>;
    max: vec3<f32>;
};

struct ChunkOutput {
    value : f32;
};

[[block]]
struct OutputBuffer {
    buffer : array<ChunkOutput>;
};

[[group(0), binding(0)]] var<uniform> chunk_info: GenerateVoxelInfo;
[[group(0), binding(1)]] var<storage, read_write> output_buffer: OutputBuffer;

// FUNCTIONS

fn inthash(x: vec3<u32>) -> vec3<f32> {
	let k = 1103515245u;
	var z: vec3<u32> = x;
    z = ((z >> vec3<u32>(8u)) ^ z.yzx)*k;
    z = ((z >> vec3<u32>(8u)) ^ z.yzx)*k;
    z = ((z >> vec3<u32>(8u)) ^ z.yzx)*k;
	let ieeeMantissa = vec3<u32>(8388607u); // 0x007FFFFF
	let ieeeOne = vec3<u32>(1065353216u); // 0x3F800000
    z = z & ieeeMantissa;
	z = z | ieeeOne;


	let f = bitcast<vec3<f32>>(z);
	return -3.0 + 2.0 * f;
}

fn precision_noise(ix: vec3<i32>, fx: vec3<f32>) -> f32 {
	let p = vec3<u32>(ix + vec3<i32>(floor(fx)));
	let w = fract(fx);
	let u = w*w*(3.0 - 2.0 * w);
	return mix( mix( mix( dot( inthash( p  ), w  ), 
                      dot( inthash( p + vec3<u32>(1u,0u,0u) ), w - vec3<f32>(1.0,0.0,0.0) ), u.x),
                 mix( dot( inthash( p + vec3<u32>(0u,1u,0u) ), w - vec3<f32>(0.0,1.0,0.0) ), 
                      dot( inthash( p + vec3<u32>(1u,1u,0u) ), w - vec3<f32>(1.0,1.0,0.0) ), u.x), u.y),
            mix( mix( dot( inthash( p + vec3<u32>(0u,0u,1u) ), w - vec3<f32>(0.0,0.0,1.0) ), 
                      dot( inthash( p + vec3<u32>(1u,0u,1u) ), w - vec3<f32>(1.0,0.0,1.0) ), u.x),
                 mix( dot( inthash( p + vec3<u32>(0u,1u,1u) ), w - vec3<f32>(0.0,1.0,1.0) ), 
                      dot( inthash( p + vec3<u32>(1u,1u,1u) ), w - vec3<f32>(1.0,1.0,1.0) ), u.x), u.y), u.z );
}

fn precision_noise_fractal(ixyz: vec3<i32>, fxyz: vec3<f32>) -> f32 {
    let period = 2;
    var octaves = 3;
    let lacunarity = 2;
    let persistence = 0.6;

    var value = 0.0;
    var curpersistence = 1.0;

    var ispace = ixyz / period;
    var fspace = vec3<f32>(ixyz - ispace * period) / vec3<f32>(f32(period)) + fxyz / vec3<f32>(f32(period));

    for (var i: i32 = 0 ; i < octaves ; i = i + 1) {
        value = value + precision_noise(ispace, fspace) * curpersistence;
        curpersistence = curpersistence * persistence;
        ispace = ispace * lacunarity;
        fspace = fspace * f32(lacunarity);
    }
    return value;
}

fn island_noise(ixyz: vec3<i32>, fxyz: vec3<f32>) -> f32 {
    return smoothStep(-0.7, 0.7, precision_noise_fractal(ixyz, fxyz));
}

fn land_noise(ixyz: vec3<i32>, fxyz: vec3<f32>) -> f32 {
    return smoothStep(-0.7, 0.7, precision_noise_fractal(ixyz + 100, vec3<f32>(fxyz.xy,1.0)));
}

fn mountain_noise(ixyz: vec3<i32>, fxyz: vec3<f32>, midpoint: f32, height: f32) -> f32 {
    let z = (fxyz.z - midpoint) / (height - midpoint);
    let land = land_noise(ixyz, fxyz);
    let mountain = smoothStep(-0.7, 0.7, precision_noise_fractal(ixyz * 10 + 1000, vec3<f32>(fxyz.xy,0.0) * 10.0));
    var noised_height = pow(z, 0.3) * ((1.0 - land) * 0.5 + 0.5);
    noised_height = smoothStep(0.0,2.0, noised_height + (noised_height * sqrt(mountain * 0.9 + 0.1) * 0.8 + 0.2));
    return 1.0 - noised_height;
}

fn index_to_point(i: u32, size: vec3<u32>) -> vec3<u32> {
    return vec3<u32>(
        i % size.x,
        (i / size.x) % size.y,
        i / (size.x * size.y)
    );
}

fn point_to_index(p: vec3<u32>, size: vec3<u32>) -> u32 {
    return p.x + size.x * (p.y + size.y * p.z);
}

// TODO: Currently workgroup_size only accept literal, change to SHADER_WORKGROUP_SIZE
// when it is fixed
[[stage(compute), workgroup_size(8u,8u,8u)]]
fn main(
    [[builtin(global_invocation_id)]] global_invocation_id: vec3<u32>,
    [[builtin(workgroup_id)]] workgroup_id: vec3<u32>,
    [[builtin(num_workgroups)]] num_workgroups: vec3<u32>
) {
    let index = point_to_index(global_invocation_id, num_workgroups * SHADER_WORKGROUP_SIZE); 
    if (index >= chunk_info.voxel_count.x * chunk_info.voxel_count.y * chunk_info.voxel_count.z) {
        return;
    }
	let point = index_to_point(index, chunk_info.voxel_count);
	let pos = mix(chunk_info.min, chunk_info.max, vec3<f32>(point) / (vec3<f32>(chunk_info.voxel_count) - 1.0));
    let midpoint = mix(chunk_info.min.z, chunk_info.max.z, f32(chunk_info.voxel_count.z / 2u) / f32(chunk_info.voxel_count.z));
    var value: f32;
    if (pos.z < midpoint) {
        value = pow(island_noise(vec3<i32>(0), pos), abs((pos.z + 0.5) * 2.0));
    } else {
        value = island_noise(vec3<i32>(0), vec3<f32>(pos.xy, midpoint)) * mountain_noise(vec3<i32>(0), pos, midpoint, chunk_info.max.z);
    }
    value = smoothStep(0.0, 1.0, value);
	output_buffer.buffer[index].value = value;
}
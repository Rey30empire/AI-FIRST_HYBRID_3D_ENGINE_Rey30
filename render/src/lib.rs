use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::time::Instant;
use thiserror::Error;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_MAP_SIZE: u32 = 2048;
const MAX_SHADOW_CASCADES: usize = 3;
const LOD_LEVEL_COUNT: usize = 3;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("failed to create render surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("failed to find a compatible graphics adapter")]
    NoAdapter,
    #[error("failed to request graphics device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

impl Vertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SceneInstance {
    pub translation: [f32; 3],
    pub bounding_radius: f32,
}

impl Default for SceneInstance {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.0, 0.0],
            bounding_radius: 1.732,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderStats {
    pub total_instances: u32,
    pub visible_instances: u32,
    pub culled_instances: u32,
    pub draw_calls_total: u32,
    pub shadow_draw_calls: u32,
    pub pbr_draw_calls: u32,
    pub tone_map_draw_calls: u32,
    pub lod_visible_counts: [u32; LOD_LEVEL_COUNT],
    pub cull_cpu_ms: f32,
    pub frame_cpu_ms: f32,
    pub gpu_buffer_mb_estimate: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct LodParams {
    pub transition_distances: [f32; 2],
    pub hysteresis: f32,
}

impl Default for LodParams {
    fn default() -> Self {
        Self {
            transition_distances: [18.0, 42.0],
            hysteresis: 3.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    translation_radius: [f32; 4],
}

impl InstanceRaw {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceRaw>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x4,
            }],
        }
    }
}

#[derive(Clone, Copy)]
struct LodSettings {
    transition_distances: [f32; LOD_LEVEL_COUNT - 1],
    hysteresis: f32,
}

impl Default for LodSettings {
    fn default() -> Self {
        Self::from(LodParams::default())
    }
}

impl From<LodParams> for LodSettings {
    fn from(value: LodParams) -> Self {
        let near = value.transition_distances[0].clamp(1.0, 5000.0);
        let far = value.transition_distances[1].clamp(near + 0.5, 5000.0);
        let hysteresis = value.hysteresis.clamp(0.0, 64.0);
        Self {
            transition_distances: [near, far],
            hysteresis,
        }
    }
}

impl From<LodSettings> for LodParams {
    fn from(value: LodSettings) -> Self {
        Self {
            transition_distances: value.transition_distances,
            hysteresis: value.hysteresis,
        }
    }
}

struct MeshLod {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    vertex_bytes: usize,
    index_bytes: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[[f32; 4]; 4]; MAX_SHADOW_CASCADES],
    camera_position: [f32; 4],
    light_direction: [f32; 4],
    light_color: [f32; 4],
    material: [f32; 4],
    base_color: [f32; 4],
    shadow_params: [f32; 4],
    cascade_splits: [f32; 4],
    ibl_sky_color: [f32; 4],
    ibl_ground_color: [f32; 4],
    ibl_params: [f32; 4],
}

impl Default for SceneUniform {
    fn default() -> Self {
        Self {
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            light_view_proj: [
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            ],
            camera_position: [0.0, 0.0, 4.0, 1.0],
            light_direction: [-0.5, -1.0, -0.3, 0.0],
            light_color: [5.5, 5.2, 5.0, 1.0],
            material: [0.2, 0.45, 1.0, 0.0],
            base_color: [0.93, 0.55, 0.36, 1.0],
            shadow_params: [0.0018, 1.0, 3.0, 0.0],
            cascade_splits: [10.0, 24.0, 1000.0, 0.0],
            ibl_sky_color: [0.65, 0.75, 0.95, 1.0],
            ibl_ground_color: [0.20, 0.18, 0.16, 1.0],
            ibl_params: [0.6, 0.0, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ToneMapUniform {
    exposure: f32,
    gamma: f32,
    bloom_intensity: f32,
    bloom_threshold: f32,
    bloom_radius: f32,
    saturation: f32,
    contrast: f32,
    white_balance: f32,
    grade_tint: [f32; 4],
    fog_color: [f32; 4],
}

impl Default for ToneMapUniform {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            gamma: 2.2,
            bloom_intensity: 0.08,
            bloom_threshold: 1.0,
            bloom_radius: 1.3,
            saturation: 1.0,
            contrast: 1.0,
            white_balance: 0.0,
            grade_tint: [1.0, 1.0, 1.0, 0.0],
            fog_color: [0.72, 0.76, 0.84, 0.0],
        }
    }
}

struct DepthTarget {
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
}

struct HdrTarget {
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
}

struct ShadowTarget {
    sample_view: wgpu::TextureView,
    cascade_views: [wgpu::TextureView; MAX_SHADOW_CASCADES],
    _texture: wgpu::Texture,
}

const CUBE_VERTICES: [Vertex; 24] = [
    Vertex {
        position: [-1.0, -1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex {
        position: [1.0, -1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex {
        position: [1.0, 1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex {
        position: [-1.0, 1.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex {
        position: [1.0, -1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
    Vertex {
        position: [-1.0, -1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
    Vertex {
        position: [-1.0, 1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
    Vertex {
        position: [1.0, 1.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
    Vertex {
        position: [-1.0, -1.0, -1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex {
        position: [-1.0, -1.0, 1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex {
        position: [-1.0, 1.0, 1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex {
        position: [-1.0, 1.0, -1.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, -1.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, -1.0, -1.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, 1.0, -1.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, 1.0, 1.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [-1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex {
        position: [1.0, 1.0, 1.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex {
        position: [1.0, 1.0, -1.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex {
        position: [-1.0, 1.0, -1.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex {
        position: [-1.0, -1.0, -1.0],
        normal: [0.0, -1.0, 0.0],
    },
    Vertex {
        position: [1.0, -1.0, -1.0],
        normal: [0.0, -1.0, 0.0],
    },
    Vertex {
        position: [1.0, -1.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    },
    Vertex {
        position: [-1.0, -1.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    },
];

const CUBE_INDICES: [u16; 36] = [
    0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17, 18,
    16, 18, 19, 20, 21, 22, 20, 22, 23,
];

const OCTAHEDRON_VERTICES: [Vertex; 6] = [
    Vertex {
        position: [0.0, 1.0, 0.0],
        normal: [0.0, 1.0, 0.0],
    },
    Vertex {
        position: [0.0, -1.0, 0.0],
        normal: [0.0, -1.0, 0.0],
    },
    Vertex {
        position: [-1.0, 0.0, 0.0],
        normal: [-1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, 0.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [0.0, 0.0, 1.0],
        normal: [0.0, 0.0, 1.0],
    },
    Vertex {
        position: [0.0, 0.0, -1.0],
        normal: [0.0, 0.0, -1.0],
    },
];

const OCTAHEDRON_INDICES: [u16; 24] = [
    0, 4, 3, 0, 3, 5, 0, 5, 2, 0, 2, 4, 1, 3, 4, 1, 5, 3, 1, 2, 5, 1, 4, 2,
];

const TETRAHEDRON_VERTICES: [Vertex; 4] = [
    Vertex {
        position: [1.0, 1.0, 1.0],
        normal: [0.57735, 0.57735, 0.57735],
    },
    Vertex {
        position: [-1.0, -1.0, 1.0],
        normal: [-0.57735, -0.57735, 0.57735],
    },
    Vertex {
        position: [-1.0, 1.0, -1.0],
        normal: [-0.57735, 0.57735, -0.57735],
    },
    Vertex {
        position: [1.0, -1.0, -1.0],
        normal: [0.57735, -0.57735, -0.57735],
    },
];

const TETRAHEDRON_INDICES: [u16; 12] = [0, 1, 2, 0, 3, 1, 0, 2, 3, 1, 3, 2];

const PBR_SHADER: &str = r#"
const PI: f32 = 3.14159265359;

struct Scene {
    view_proj: mat4x4<f32>,
    light_view_proj: array<mat4x4<f32>, 3>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_color: vec4<f32>,
    material: vec4<f32>,
    base_color: vec4<f32>,
    shadow_params: vec4<f32>,
    cascade_splits: vec4<f32>,
    ibl_sky_color: vec4<f32>,
    ibl_ground_color: vec4<f32>,
    ibl_params: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> scene: Scene;

@group(1) @binding(0)
var shadow_map: texture_depth_2d_array;

@group(1) @binding(1)
var shadow_sampler: sampler_comparison;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) instance_translation_radius: vec4<f32>
) -> VsOut {
    var out: VsOut;
    let world_pos = position + instance_translation_radius.xyz;
    let world = vec4<f32>(world_pos, 1.0);
    out.position = scene.view_proj * world;
    out.world_pos = world_pos;
    out.normal = normal;
    return out;
}

fn distribution_ggx(n: vec3<f32>, h: vec3<f32>, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let n_dot_h = max(dot(n, h), 0.0);
    let n_dot_h2 = n_dot_h * n_dot_h;
    let num = a2;
    let denom = (n_dot_h2 * (a2 - 1.0) + 1.0);
    return num / max(PI * denom * denom, 0.000001);
}

fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    let num = n_dot_v;
    let denom = n_dot_v * (1.0 - k) + k;
    return num / max(denom, 0.000001);
}

fn geometry_smith(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, roughness: f32) -> f32 {
    let n_dot_v = max(dot(n, v), 0.0);
    let n_dot_l = max(dot(n, l), 0.0);
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - cos_theta, 5.0);
}

fn sample_shadow(world_pos: vec3<f32>, cascade_index: i32, bias: f32) -> f32 {
    let light_clip = scene.light_view_proj[cascade_index] * vec4<f32>(world_pos, 1.0);
    let proj = light_clip.xyz / max(light_clip.w, 0.0001);
    let uv = proj.xy * 0.5 + vec2<f32>(0.5, 0.5);
    let depth = proj.z * 0.5 + 0.5;

    if (uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || depth <= 0.0 || depth >= 1.0) {
        return 1.0;
    }

    let tex_size = vec2<f32>(textureDimensions(shadow_map));
    let texel = 1.0 / tex_size;
    var sum = 0.0;
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>(-0.75, -0.75), cascade_index, depth - bias);
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>( 0.75, -0.75), cascade_index, depth - bias);
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>(-0.75,  0.75), cascade_index, depth - bias);
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>( 0.75,  0.75), cascade_index, depth - bias);
    return sum * 0.25;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let albedo = scene.base_color.xyz;
    let metallic = clamp(scene.material.x, 0.0, 1.0);
    let roughness = clamp(scene.material.y, 0.04, 1.0);
    let ao = clamp(scene.material.z, 0.0, 1.0);

    let n = normalize(in.normal);
    let v = normalize(scene.camera_position.xyz - in.world_pos);
    let l = normalize(-scene.light_direction.xyz);
    let h = normalize(v + l);

    let f0 = mix(vec3<f32>(0.04, 0.04, 0.04), albedo, vec3<f32>(metallic));
    let ndf = distribution_ggx(n, h, roughness);
    let g = geometry_smith(n, v, l, roughness);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);

    let numerator = ndf * g * f;
    let denominator = max(4.0 * max(dot(n, v), 0.0) * max(dot(n, l), 0.0), 0.000001);
    let specular = numerator / denominator;

    let ks = f;
    let kd = (vec3<f32>(1.0) - ks) * (1.0 - metallic);
    let n_dot_l = max(dot(n, l), 0.0);

    let view_depth = distance(in.world_pos, scene.camera_position.xyz);
    let cascade_count = i32(clamp(scene.shadow_params.z, 1.0, 3.0));
    var cascade_index = 0i;
    if (cascade_count > 1 && view_depth > scene.cascade_splits.x) {
        cascade_index = 1;
    }
    if (cascade_count > 2 && view_depth > scene.cascade_splits.y) {
        cascade_index = 2;
    }

    let shadow = sample_shadow(in.world_pos, cascade_index, scene.shadow_params.x) * scene.shadow_params.y;
    let radiance = scene.light_color.xyz;
    let lo = (kd * albedo / PI + specular) * radiance * n_dot_l * shadow;
    let hemi = mix(scene.ibl_ground_color.xyz, scene.ibl_sky_color.xyz, clamp(n.y * 0.5 + 0.5, 0.0, 1.0));
    let ambient = hemi * albedo * ao * scene.ibl_params.x;

    let hdr_linear = ambient + lo;
    return vec4<f32>(hdr_linear, 1.0);
}
"#;

const SHADOW_SHADER: &str = r#"
struct Scene {
    view_proj: mat4x4<f32>,
    light_view_proj: array<mat4x4<f32>, 3>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_color: vec4<f32>,
    material: vec4<f32>,
    base_color: vec4<f32>,
    shadow_params: vec4<f32>,
    cascade_splits: vec4<f32>,
    ibl_sky_color: vec4<f32>,
    ibl_ground_color: vec4<f32>,
    ibl_params: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> scene: Scene;

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) _normal: vec3<f32>,
    @location(2) instance_translation_radius: vec4<f32>
) -> @builtin(position) vec4<f32> {
    let cascade_index = i32(clamp(scene.shadow_params.w, 0.0, 2.0));
    return scene.light_view_proj[cascade_index] * vec4<f32>(position + instance_translation_radius.xyz, 1.0);
}
"#;

const TONEMAP_SHADER: &str = r#"
struct ToneMap {
    exposure: f32,
    gamma: f32,
    bloom_intensity: f32,
    bloom_threshold: f32,
    bloom_radius: f32,
    saturation: f32,
    contrast: f32,
    white_balance: f32,
    grade_tint: vec4<f32>,
    fog_color: vec4<f32>,
}

@group(0) @binding(0)
var hdr_tex: texture_2d<f32>;

@group(0) @binding(1)
var<uniform> tone: ToneMap;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    var position = vec2<f32>(3.0, 1.0);
    var uv = vec2<f32>(2.0, 0.0);
    if (vertex_index == 0u) {
        position = vec2<f32>(-1.0, -3.0);
        uv = vec2<f32>(0.0, 2.0);
    } else if (vertex_index == 1u) {
        position = vec2<f32>(-1.0, 1.0);
        uv = vec2<f32>(0.0, 0.0);
    }
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn sample_hdr(uv: vec2<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(hdr_tex));
    let clamped_uv = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let texel = vec2<i32>(clamp(clamped_uv * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));
    return textureLoad(hdr_tex, texel, 0).rgb;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let hdr_base = sample_hdr(in.uv);
    let texel_step = 1.0 / vec2<f32>(textureDimensions(hdr_tex));
    let radius = max(tone.bloom_radius, 0.1);
    var bloom = vec3<f32>(0.0);
    bloom += max(sample_hdr(in.uv + texel_step * vec2<f32>( radius,  0.0)) - vec3<f32>(tone.bloom_threshold), vec3<f32>(0.0));
    bloom += max(sample_hdr(in.uv + texel_step * vec2<f32>(-radius,  0.0)) - vec3<f32>(tone.bloom_threshold), vec3<f32>(0.0));
    bloom += max(sample_hdr(in.uv + texel_step * vec2<f32>( 0.0,  radius)) - vec3<f32>(tone.bloom_threshold), vec3<f32>(0.0));
    bloom += max(sample_hdr(in.uv + texel_step * vec2<f32>( 0.0, -radius)) - vec3<f32>(tone.bloom_threshold), vec3<f32>(0.0));
    bloom *= 0.25 * max(tone.bloom_intensity, 0.0);

    var hdr = (hdr_base + bloom) * tone.exposure;
    let wb = clamp(tone.white_balance, -1.0, 1.0);
    hdr *= vec3<f32>(1.0 + wb * 0.15, 1.0, 1.0 - wb * 0.15);
    hdr *= tone.grade_tint.rgb;

    let luma = dot(hdr, vec3<f32>(0.2126, 0.7152, 0.0722));
    let graded_sat = mix(vec3<f32>(luma), hdr, clamp(tone.saturation, 0.0, 2.0));
    let graded_contrast = (graded_sat - vec3<f32>(0.5)) * clamp(tone.contrast, 0.5, 2.0) + vec3<f32>(0.5);
    let fogged = mix(graded_contrast, tone.fog_color.rgb, clamp(tone.fog_color.a, 0.0, 1.0));

    let mapped = fogged / (fogged + vec3<f32>(1.0));
    let gamma_corrected = pow(mapped, vec3<f32>(1.0 / max(tone.gamma, 0.001)));
    return vec4<f32>(gamma_corrected, 1.0);
}
"#;

pub struct Renderer<'window> {
    surface: wgpu::Surface<'window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,

    scene_uniform: SceneUniform,
    scene_buffer: wgpu::Buffer,
    scene_bind_group: wgpu::BindGroup,
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_bind_group: wgpu::BindGroup,
    pbr_pipeline: wgpu::RenderPipeline,

    tone_map_uniform: ToneMapUniform,
    tone_map_buffer: wgpu::Buffer,
    tone_map_bind_group_layout: wgpu::BindGroupLayout,
    tone_map_bind_group: wgpu::BindGroup,
    tone_map_pipeline: wgpu::RenderPipeline,

    lod_meshes: [MeshLod; LOD_LEVEL_COUNT],
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    scene_instances: Vec<SceneInstance>,
    instance_lod_levels: Vec<u8>,
    visible_instances: Vec<InstanceRaw>,
    visible_instances_by_lod: [Vec<InstanceRaw>; LOD_LEVEL_COUNT],
    visible_instance_counts: [u32; LOD_LEVEL_COUNT],
    lod_settings: LodSettings,
    last_stats: RenderStats,

    hdr_target: HdrTarget,
    depth_target: DepthTarget,
    shadow_target: ShadowTarget,
}

impl<'window> Renderer<'window> {
    pub async fn new(window: &'window Window) -> Result<Self, RenderError> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(RenderError::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("ai-first-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);
        let present_mode = caps
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Fifo)
            .unwrap_or(caps.present_modes[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let mut scene_uniform = SceneUniform::default();
        let cascades = compute_shadow_cascades(
            [
                scene_uniform.light_direction[0],
                scene_uniform.light_direction[1],
                scene_uniform.light_direction[2],
            ],
            [
                scene_uniform.camera_position[0],
                scene_uniform.camera_position[1],
                scene_uniform.camera_position[2],
            ],
            scene_uniform.shadow_params[2] as u32,
        );
        scene_uniform.light_view_proj = cascades.matrices;
        scene_uniform.cascade_splits = cascades.splits;
        let scene_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene-buffer"),
            contents: bytemuck::bytes_of(&scene_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scene_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene-bind-group-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene-bind-group"),
            layout: &scene_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_buffer.as_entire_binding(),
            }],
        });

        let shadow_target = Self::create_shadow_target(&device);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..wgpu::SamplerDescriptor::default()
        });
        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });
        let shadow_bind_group = Self::create_shadow_bind_group(
            &device,
            &shadow_bind_group_layout,
            &shadow_target.sample_view,
            &shadow_sampler,
        );

        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADOW_SHADER.into()),
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow-layout"),
                bind_group_layouts: &[&scene_bind_group_layout],
                push_constant_ranges: &[],
            });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow-pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: "vs_main",
                buffers: &[Vertex::layout(), InstanceRaw::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let pbr_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-hdr-shader"),
            source: wgpu::ShaderSource::Wgsl(PBR_SHADER.into()),
        });
        let pbr_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr-layout"),
            bind_group_layouts: &[&scene_bind_group_layout, &shadow_bind_group_layout],
            push_constant_ranges: &[],
        });
        let pbr_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr-pipeline"),
            layout: Some(&pbr_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &pbr_shader,
                entry_point: "vs_main",
                buffers: &[Vertex::layout(), InstanceRaw::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &pbr_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let tone_map_uniform = ToneMapUniform::default();
        let tone_map_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tone-map-buffer"),
            contents: bytemuck::bytes_of(&tone_map_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let tone_map_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tone-map-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let hdr_target = Self::create_hdr_target(&device, &config);
        let depth_target = Self::create_depth_target(&device, &config);
        let tone_map_bind_group = Self::create_tone_map_bind_group(
            &device,
            &tone_map_bind_group_layout,
            &hdr_target.view,
            &tone_map_buffer,
        );

        let tone_map_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tone-map-shader"),
            source: wgpu::ShaderSource::Wgsl(TONEMAP_SHADER.into()),
        });
        let tone_map_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tone-map-layout"),
                bind_group_layouts: &[&tone_map_bind_group_layout],
                push_constant_ranges: &[],
            });
        let tone_map_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tone-map-pipeline"),
            layout: Some(&tone_map_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &tone_map_shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &tone_map_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let lod_meshes = [
            Self::create_mesh_lod(&device, "lod0-cube", &CUBE_VERTICES, &CUBE_INDICES),
            Self::create_mesh_lod(
                &device,
                "lod1-octa",
                &OCTAHEDRON_VERTICES,
                &OCTAHEDRON_INDICES,
            ),
            Self::create_mesh_lod(
                &device,
                "lod2-tetra",
                &TETRAHEDRON_VERTICES,
                &TETRAHEDRON_INDICES,
            ),
        ];
        let default_instances = vec![SceneInstance::default()];
        let default_lod_levels = vec![0u8; default_instances.len()];
        let default_visible = default_instances
            .iter()
            .map(|instance| InstanceRaw {
                translation_radius: [
                    instance.translation[0],
                    instance.translation[1],
                    instance.translation[2],
                    instance.bounding_radius.max(0.1),
                ],
            })
            .collect::<Vec<InstanceRaw>>();
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instance-buffer"),
            contents: bytemuck::cast_slice(&default_visible),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            scene_uniform,
            scene_buffer,
            scene_bind_group,
            shadow_pipeline,
            shadow_bind_group,
            pbr_pipeline,
            tone_map_uniform,
            tone_map_buffer,
            tone_map_bind_group_layout,
            tone_map_bind_group,
            tone_map_pipeline,
            lod_meshes,
            instance_buffer,
            instance_capacity: default_visible.len().max(1),
            scene_instances: default_instances,
            instance_lod_levels: default_lod_levels,
            visible_instances: default_visible,
            visible_instances_by_lod: [Vec::new(), Vec::new(), Vec::new()],
            visible_instance_counts: [0; LOD_LEVEL_COUNT],
            lod_settings: LodSettings::default(),
            last_stats: RenderStats::default(),
            hdr_target,
            depth_target,
            shadow_target,
        })
    }

    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }

        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);

        self.depth_target = Self::create_depth_target(&self.device, &self.config);
        self.hdr_target = Self::create_hdr_target(&self.device, &self.config);
        self.tone_map_bind_group = Self::create_tone_map_bind_group(
            &self.device,
            &self.tone_map_bind_group_layout,
            &self.hdr_target.view,
            &self.tone_map_buffer,
        );
    }

    pub fn set_scene_instances(&mut self, instances: &[SceneInstance]) {
        if instances.is_empty() {
            self.scene_instances = vec![SceneInstance::default()];
        } else {
            self.scene_instances = instances.to_vec();
        }
        self.instance_lod_levels
            .resize(self.scene_instances.len(), 0u8);
    }

    pub fn stats(&self) -> RenderStats {
        self.last_stats
    }

    pub fn set_lod_settings(&mut self, params: LodParams) {
        self.lod_settings = params.into();
    }

    pub fn lod_settings(&self) -> LodParams {
        self.lod_settings.into()
    }

    pub fn update_camera(&mut self, view_proj: [[f32; 4]; 4], camera_position: [f32; 3]) {
        self.scene_uniform.view_proj = view_proj;
        self.scene_uniform.camera_position = [
            camera_position[0],
            camera_position[1],
            camera_position[2],
            1.0,
        ];
        let cascades = compute_shadow_cascades(
            [
                self.scene_uniform.light_direction[0],
                self.scene_uniform.light_direction[1],
                self.scene_uniform.light_direction[2],
            ],
            camera_position,
            self.scene_uniform.shadow_params[2] as u32,
        );
        self.scene_uniform.light_view_proj = cascades.matrices;
        self.scene_uniform.cascade_splits = cascades.splits;
        self.queue.write_buffer(
            &self.scene_buffer,
            0,
            bytemuck::bytes_of(&self.scene_uniform),
        );
    }

    pub fn set_directional_light(&mut self, params: DirectionalLightParams) {
        let direction = normalize_direction(params.direction);
        let intensity = params.intensity.clamp(0.01, 32.0);
        self.scene_uniform.light_direction = [direction[0], direction[1], direction[2], 0.0];
        self.scene_uniform.light_color = [
            params.color[0].clamp(0.0, 32.0) * intensity,
            params.color[1].clamp(0.0, 32.0) * intensity,
            params.color[2].clamp(0.0, 32.0) * intensity,
            1.0,
        ];
        self.scene_uniform.shadow_params[0] = params.shadow_bias.clamp(0.0001, 0.01);
        self.scene_uniform.shadow_params[1] = params.shadow_strength.clamp(0.0, 1.0);
        self.scene_uniform.shadow_params[2] = (params
            .shadow_cascade_count
            .clamp(1, MAX_SHADOW_CASCADES as u32))
            as f32;
        let cascades = compute_shadow_cascades(
            direction,
            [
                self.scene_uniform.camera_position[0],
                self.scene_uniform.camera_position[1],
                self.scene_uniform.camera_position[2],
            ],
            self.scene_uniform.shadow_params[2] as u32,
        );
        self.scene_uniform.light_view_proj = cascades.matrices;
        self.scene_uniform.cascade_splits = cascades.splits;
        self.queue.write_buffer(
            &self.scene_buffer,
            0,
            bytemuck::bytes_of(&self.scene_uniform),
        );
    }

    pub fn set_ibl(&mut self, params: IblParams) {
        self.scene_uniform.ibl_sky_color = [
            params.sky_color[0].clamp(0.0, 4.0),
            params.sky_color[1].clamp(0.0, 4.0),
            params.sky_color[2].clamp(0.0, 4.0),
            1.0,
        ];
        self.scene_uniform.ibl_ground_color = [
            params.ground_color[0].clamp(0.0, 4.0),
            params.ground_color[1].clamp(0.0, 4.0),
            params.ground_color[2].clamp(0.0, 4.0),
            1.0,
        ];
        self.scene_uniform.ibl_params[0] = params.intensity.clamp(0.0, 4.0);
        self.queue.write_buffer(
            &self.scene_buffer,
            0,
            bytemuck::bytes_of(&self.scene_uniform),
        );
    }

    pub fn set_postprocess(&mut self, params: ToneMapParams) {
        self.tone_map_uniform.exposure = params.exposure.clamp(0.05, 16.0);
        self.tone_map_uniform.gamma = params.gamma.clamp(0.2, 4.0);
        self.tone_map_uniform.bloom_intensity = params.bloom_intensity.clamp(0.0, 4.0);
        self.tone_map_uniform.bloom_threshold = params.bloom_threshold.clamp(0.0, 8.0);
        self.tone_map_uniform.bloom_radius = params.bloom_radius.clamp(0.1, 8.0);
        self.tone_map_uniform.saturation = params.saturation.clamp(0.0, 2.0);
        self.tone_map_uniform.contrast = params.contrast.clamp(0.5, 2.0);
        self.tone_map_uniform.white_balance = params.white_balance.clamp(-1.0, 1.0);
        self.tone_map_uniform.grade_tint = [
            params.grade_tint[0].clamp(0.0, 2.0),
            params.grade_tint[1].clamp(0.0, 2.0),
            params.grade_tint[2].clamp(0.0, 2.0),
            0.0,
        ];
        self.tone_map_uniform.fog_color = [
            params.fog_color[0].clamp(0.0, 4.0),
            params.fog_color[1].clamp(0.0, 4.0),
            params.fog_color[2].clamp(0.0, 4.0),
            params.fog_density.clamp(0.0, 1.0),
        ];
        self.queue.write_buffer(
            &self.tone_map_buffer,
            0,
            bytemuck::bytes_of(&self.tone_map_uniform),
        );
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let frame_started = Instant::now();
        let cull_started = Instant::now();
        self.prepare_visible_instances();
        let cull_cpu_ms = cull_started.elapsed().as_secs_f32() * 1000.0;
        let instance_count = self.visible_instances.len() as u32;

        let frame = self.surface.get_current_texture()?;
        let swapchain_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("main-render-encoder"),
            });

        let cascade_count = (self.scene_uniform.shadow_params[2] as u32)
            .clamp(1, MAX_SHADOW_CASCADES as u32) as usize;
        for cascade_index in 0..cascade_count {
            self.scene_uniform.shadow_params[3] = cascade_index as f32;
            self.queue.write_buffer(
                &self.scene_buffer,
                0,
                bytemuck::bytes_of(&self.scene_uniform),
            );
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_target.cascade_views[cascade_index],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            shadow_pass.set_pipeline(&self.shadow_pipeline);
            shadow_pass.set_bind_group(0, &self.scene_bind_group, &[]);
            if instance_count > 0 {
                let mut instance_base = 0u32;
                for lod_index in 0..LOD_LEVEL_COUNT {
                    let lod_count = self.visible_instance_counts[lod_index];
                    if lod_count == 0 {
                        continue;
                    }
                    let lod_mesh = &self.lod_meshes[lod_index];
                    shadow_pass.set_vertex_buffer(0, lod_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        lod_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    shadow_pass.draw_indexed(
                        0..lod_mesh.index_count,
                        0,
                        instance_base..instance_base + lod_count,
                    );
                    instance_base += lod_count;
                }
            }
        }
        self.scene_uniform.shadow_params[3] = 0.0;
        self.queue.write_buffer(
            &self.scene_buffer,
            0,
            bytemuck::bytes_of(&self.scene_uniform),
        );

        {
            let mut pbr_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pbr-hdr-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_target.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pbr_pass.set_pipeline(&self.pbr_pipeline);
            pbr_pass.set_bind_group(0, &self.scene_bind_group, &[]);
            pbr_pass.set_bind_group(1, &self.shadow_bind_group, &[]);
            if instance_count > 0 {
                let mut instance_base = 0u32;
                for lod_index in 0..LOD_LEVEL_COUNT {
                    let lod_count = self.visible_instance_counts[lod_index];
                    if lod_count == 0 {
                        continue;
                    }
                    let lod_mesh = &self.lod_meshes[lod_index];
                    pbr_pass.set_vertex_buffer(0, lod_mesh.vertex_buffer.slice(..));
                    pbr_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                    pbr_pass.set_index_buffer(
                        lod_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pbr_pass.draw_indexed(
                        0..lod_mesh.index_count,
                        0,
                        instance_base..instance_base + lod_count,
                    );
                    instance_base += lod_count;
                }
            }
        }

        {
            let mut tone_map_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone-map-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &swapchain_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.01,
                            g: 0.02,
                            b: 0.03,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            tone_map_pass.set_pipeline(&self.tone_map_pipeline);
            tone_map_pass.set_bind_group(0, &self.tone_map_bind_group, &[]);
            tone_map_pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        let lod_bucket_count = self
            .visible_instance_counts
            .iter()
            .copied()
            .filter(|count| *count > 0)
            .count() as u32;
        let shadow_draw_calls = if instance_count > 0 {
            lod_bucket_count * cascade_count as u32
        } else {
            0
        };
        let pbr_draw_calls = if instance_count > 0 {
            lod_bucket_count
        } else {
            0
        };
        let tone_map_draw_calls = 1;
        let draw_calls_total = shadow_draw_calls + pbr_draw_calls + tone_map_draw_calls;
        let total_instances = self.scene_instances.len() as u32;
        let visible_instances = instance_count;
        self.last_stats = RenderStats {
            total_instances,
            visible_instances,
            culled_instances: total_instances.saturating_sub(visible_instances),
            draw_calls_total,
            shadow_draw_calls,
            pbr_draw_calls,
            tone_map_draw_calls,
            lod_visible_counts: self.visible_instance_counts,
            cull_cpu_ms,
            frame_cpu_ms: frame_started.elapsed().as_secs_f32() * 1000.0,
            gpu_buffer_mb_estimate: self.gpu_buffer_bytes_estimate() as f32 / (1024.0 * 1024.0),
        };
        Ok(())
    }

    fn prepare_visible_instances(&mut self) {
        let frustum_planes = extract_frustum_planes(self.scene_uniform.view_proj);
        let camera_position = [
            self.scene_uniform.camera_position[0],
            self.scene_uniform.camera_position[1],
            self.scene_uniform.camera_position[2],
        ];
        self.visible_instances.clear();
        for bucket in &mut self.visible_instances_by_lod {
            bucket.clear();
            bucket.reserve(self.scene_instances.len() / LOD_LEVEL_COUNT + 1);
        }
        self.visible_instance_counts = [0; LOD_LEVEL_COUNT];

        for (index, instance) in self.scene_instances.iter().copied().enumerate() {
            let radius = instance.bounding_radius.max(0.1);
            if sphere_in_frustum(instance.translation, radius, &frustum_planes) {
                let distance = Vec3::from_array(instance.translation)
                    .distance(Vec3::from_array(camera_position));
                let current_lod = self.instance_lod_levels.get(index).copied().unwrap_or(0);
                let next_lod = select_lod_level(distance, current_lod, &self.lod_settings);
                if let Some(level) = self.instance_lod_levels.get_mut(index) {
                    *level = next_lod;
                }
                self.visible_instances_by_lod[next_lod as usize].push(InstanceRaw {
                    translation_radius: [
                        instance.translation[0],
                        instance.translation[1],
                        instance.translation[2],
                        radius,
                    ],
                });
            }
        }

        for lod_index in 0..LOD_LEVEL_COUNT {
            self.visible_instance_counts[lod_index] =
                self.visible_instances_by_lod[lod_index].len() as u32;
            self.visible_instances
                .extend_from_slice(&self.visible_instances_by_lod[lod_index]);
        }

        let visible_len = self.visible_instances.len();
        if visible_len == 0 {
            return;
        }
        self.ensure_instance_buffer_capacity(visible_len);
        self.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&self.visible_instances),
        );
    }

    fn ensure_instance_buffer_capacity(&mut self, required_instances: usize) {
        if required_instances <= self.instance_capacity {
            return;
        }
        let new_capacity = required_instances.next_power_of_two().max(1);
        let byte_size = (new_capacity * std::mem::size_of::<InstanceRaw>()) as wgpu::BufferAddress;
        self.instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance-buffer"),
            size: byte_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }

    fn gpu_buffer_bytes_estimate(&self) -> usize {
        let lod_bytes = self
            .lod_meshes
            .iter()
            .map(|lod| lod.vertex_bytes + lod.index_bytes)
            .sum::<usize>();
        let instance_bytes = self.instance_capacity * std::mem::size_of::<InstanceRaw>();
        lod_bytes + instance_bytes
    }

    fn create_mesh_lod(
        device: &wgpu::Device,
        label: &str,
        vertices: &[Vertex],
        indices: &[u16],
    ) -> MeshLod {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-vertex-buffer", label)),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-index-buffer", label)),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        MeshLod {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            vertex_bytes: std::mem::size_of_val(vertices),
            index_bytes: std::mem::size_of_val(indices),
        }
    }

    fn create_depth_target(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> DepthTarget {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth-texture"),
            size: wgpu::Extent3d {
                width: config.width.max(1),
                height: config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        DepthTarget {
            view,
            _texture: texture,
        }
    }

    fn create_hdr_target(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> HdrTarget {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr-texture"),
            size: wgpu::Extent3d {
                width: config.width.max(1),
                height: config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        HdrTarget {
            view,
            _texture: texture,
        }
    }

    fn create_shadow_target(device: &wgpu::Device) -> ShadowTarget {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-map-texture"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: MAX_SHADOW_CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let sample_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow-map-array-view"),
            format: Some(SHADOW_FORMAT),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(MAX_SHADOW_CASCADES as u32),
        });
        let cascade_views = std::array::from_fn(|cascade_index| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("shadow-map-cascade-view"),
                format: Some(SHADOW_FORMAT),
                dimension: Some(wgpu::TextureViewDimension::D2),
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: Some(1),
                base_array_layer: cascade_index as u32,
                array_layer_count: Some(1),
            })
        });
        ShadowTarget {
            sample_view,
            cascade_views,
            _texture: texture,
        }
    }

    fn create_shadow_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        shadow_view: &wgpu::TextureView,
        shadow_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
            ],
        })
    }

    fn create_tone_map_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        tone_map_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tone-map-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tone_map_buffer.as_entire_binding(),
                },
            ],
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DirectionalLightParams {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub shadow_bias: f32,
    pub shadow_strength: f32,
    pub shadow_cascade_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct IblParams {
    pub sky_color: [f32; 3],
    pub ground_color: [f32; 3],
    pub intensity: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ToneMapParams {
    pub exposure: f32,
    pub gamma: f32,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub bloom_radius: f32,
    pub fog_density: f32,
    pub fog_color: [f32; 3],
    pub saturation: f32,
    pub contrast: f32,
    pub white_balance: f32,
    pub grade_tint: [f32; 3],
}

impl Default for ToneMapParams {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            gamma: 2.2,
            bloom_intensity: 0.08,
            bloom_threshold: 1.0,
            bloom_radius: 1.3,
            fog_density: 0.0,
            fog_color: [0.72, 0.76, 0.84],
            saturation: 1.0,
            contrast: 1.0,
            white_balance: 0.0,
            grade_tint: [1.0, 1.0, 1.0],
        }
    }
}

#[derive(Clone, Copy)]
struct FrustumPlane {
    normal: Vec3,
    d: f32,
}

fn extract_frustum_planes(view_proj: [[f32; 4]; 4]) -> [FrustumPlane; 6] {
    let matrix = Mat4::from_cols_array_2d(&view_proj);
    let cols = matrix.to_cols_array_2d();
    let row0 = Vec4::new(cols[0][0], cols[1][0], cols[2][0], cols[3][0]);
    let row1 = Vec4::new(cols[0][1], cols[1][1], cols[2][1], cols[3][1]);
    let row2 = Vec4::new(cols[0][2], cols[1][2], cols[2][2], cols[3][2]);
    let row3 = Vec4::new(cols[0][3], cols[1][3], cols[2][3], cols[3][3]);

    [
        normalize_plane(row3 + row0),
        normalize_plane(row3 - row0),
        normalize_plane(row3 + row1),
        normalize_plane(row3 - row1),
        normalize_plane(row3 + row2),
        normalize_plane(row3 - row2),
    ]
}

fn normalize_plane(raw: Vec4) -> FrustumPlane {
    let normal = raw.truncate();
    let inv_len = normal.length().max(1e-6).recip();
    FrustumPlane {
        normal: normal * inv_len,
        d: raw.w * inv_len,
    }
}

fn sphere_in_frustum(center: [f32; 3], radius: f32, planes: &[FrustumPlane; 6]) -> bool {
    let center = Vec3::from_array(center);
    planes
        .iter()
        .all(|plane| plane.normal.dot(center) + plane.d + radius >= 0.0)
}

fn select_lod_level(distance: f32, current_lod: u8, settings: &LodSettings) -> u8 {
    let near = settings.transition_distances[0];
    let far = settings.transition_distances[1];
    let hysteresis = settings.hysteresis.max(0.0);

    match current_lod.min((LOD_LEVEL_COUNT - 1) as u8) {
        0 => {
            if distance > near + hysteresis {
                if distance > far + hysteresis { 2 } else { 1 }
            } else {
                0
            }
        }
        1 => {
            if distance > far + hysteresis {
                2
            } else if distance < near - hysteresis {
                0
            } else {
                1
            }
        }
        _ => {
            if distance < far - hysteresis {
                if distance < near - hysteresis { 0 } else { 1 }
            } else {
                2
            }
        }
    }
}

struct ShadowCascadeData {
    matrices: [[[f32; 4]; 4]; MAX_SHADOW_CASCADES],
    splits: [f32; 4],
}

fn compute_shadow_cascades(
    light_direction: [f32; 3],
    camera_position: [f32; 3],
    cascade_count: u32,
) -> ShadowCascadeData {
    let dir = Vec3::from_array(normalize_direction(light_direction));
    let focus = Vec3::from_array(camera_position) * 0.35;
    let clamped_count = cascade_count.clamp(1, MAX_SHADOW_CASCADES as u32) as usize;

    let (radii, splits) = match clamped_count {
        1 => ([42.0, 42.0, 42.0], [1000.0, 1000.0, 1000.0, 0.0]),
        2 => ([14.0, 42.0, 42.0], [16.0, 1000.0, 1000.0, 0.0]),
        _ => ([10.0, 24.0, 56.0], [12.0, 30.0, 1000.0, 0.0]),
    };
    let matrices = std::array::from_fn(|cascade_index| {
        compute_light_view_proj_for_cascade(dir, focus, radii[cascade_index])
    });
    ShadowCascadeData { matrices, splits }
}

fn compute_light_view_proj_for_cascade(
    light_dir: Vec3,
    focus_center: Vec3,
    cascade_radius: f32,
) -> [[f32; 4]; 4] {
    let eye = focus_center - light_dir * (cascade_radius * 1.8);
    let up = if light_dir.y.abs() > 0.98 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let view = Mat4::look_at_rh(eye, focus_center, up);
    let proj = Mat4::orthographic_rh_gl(
        -cascade_radius,
        cascade_radius,
        -cascade_radius,
        cascade_radius,
        0.1,
        cascade_radius * 5.0,
    );
    (proj * view).to_cols_array_2d()
}

fn normalize_direction(direction: [f32; 3]) -> [f32; 3] {
    let mut dir = Vec3::from_array(direction);
    if dir.length_squared() < 1e-6 {
        dir = Vec3::new(-0.5, -1.0, -0.3);
    } else {
        dir = dir.normalize();
    }
    dir.to_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_hysteresis_promotes_and_demotes_stably() {
        let settings = LodSettings::default();

        assert_eq!(select_lod_level(10.0, 0, &settings), 0);
        assert_eq!(select_lod_level(24.0, 0, &settings), 1);
        assert_eq!(select_lod_level(44.0, 1, &settings), 1);
        assert_eq!(select_lod_level(46.0, 1, &settings), 2);
        assert_eq!(select_lod_level(40.0, 2, &settings), 2);
        assert_eq!(select_lod_level(38.0, 2, &settings), 1);
        assert_eq!(select_lod_level(13.0, 1, &settings), 0);
    }

    #[test]
    fn lod_selector_handles_large_distance_jump() {
        let settings = LodSettings::default();

        assert_eq!(select_lod_level(100.0, 0, &settings), 2);
        assert_eq!(select_lod_level(2.0, 2, &settings), 0);
    }

    #[test]
    fn lod_params_are_sanitized_when_applied() {
        let settings = LodSettings::from(LodParams {
            transition_distances: [12.0, 8.0],
            hysteresis: -2.0,
        });
        assert!(settings.transition_distances[0] < settings.transition_distances[1]);
        assert_eq!(settings.hysteresis, 0.0);
    }
}

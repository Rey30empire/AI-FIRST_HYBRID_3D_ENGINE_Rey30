use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use thiserror::Error;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_MAP_SIZE: u32 = 2048;

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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[f32; 4]; 4],
    camera_position: [f32; 4],
    light_direction: [f32; 4],
    light_color: [f32; 4],
    material: [f32; 4],
    base_color: [f32; 4],
    shadow_params: [f32; 4],
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
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            camera_position: [0.0, 0.0, 4.0, 1.0],
            light_direction: [-0.5, -1.0, -0.3, 0.0],
            light_color: [5.5, 5.2, 5.0, 1.0],
            material: [0.2, 0.45, 1.0, 0.0],
            base_color: [0.93, 0.55, 0.36, 1.0],
            shadow_params: [0.0018, 1.0, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ToneMapUniform {
    exposure: f32,
    gamma: f32,
    _pad0: f32,
    _pad1: f32,
}

impl Default for ToneMapUniform {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            gamma: 2.2,
            _pad0: 0.0,
            _pad1: 0.0,
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
    view: wgpu::TextureView,
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

const PBR_SHADER: &str = r#"
const PI: f32 = 3.14159265359;

struct Scene {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_color: vec4<f32>,
    material: vec4<f32>,
    base_color: vec4<f32>,
    shadow_params: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> scene: Scene;

@group(1) @binding(0)
var shadow_map: texture_depth_2d;

@group(1) @binding(1)
var shadow_sampler: sampler_comparison;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) light_clip: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> VsOut {
    var out: VsOut;
    let world = vec4<f32>(position, 1.0);
    out.position = scene.view_proj * world;
    out.world_pos = position;
    out.normal = normal;
    out.light_clip = scene.light_view_proj * world;
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

fn sample_shadow(light_clip: vec4<f32>, bias: f32) -> f32 {
    let proj = light_clip.xyz / max(light_clip.w, 0.0001);
    let uv = proj.xy * 0.5 + vec2<f32>(0.5, 0.5);
    let depth = proj.z * 0.5 + 0.5;

    if (uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || depth <= 0.0 || depth >= 1.0) {
        return 1.0;
    }

    let tex_size = vec2<f32>(textureDimensions(shadow_map));
    let texel = 1.0 / tex_size;
    var sum = 0.0;
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>(-0.75, -0.75), depth - bias);
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>( 0.75, -0.75), depth - bias);
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>(-0.75,  0.75), depth - bias);
    sum += textureSampleCompare(shadow_map, shadow_sampler, uv + texel * vec2<f32>( 0.75,  0.75), depth - bias);
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

    let shadow = sample_shadow(in.light_clip, scene.shadow_params.x) * scene.shadow_params.y;
    let radiance = scene.light_color.xyz;
    let lo = (kd * albedo / PI + specular) * radiance * n_dot_l * shadow;
    let ambient = vec3<f32>(0.03, 0.03, 0.03) * albedo * ao;

    let hdr_linear = ambient + lo;
    return vec4<f32>(hdr_linear, 1.0);
}
"#;

const SHADOW_SHADER: &str = r#"
struct Scene {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_position: vec4<f32>,
    light_direction: vec4<f32>,
    light_color: vec4<f32>,
    material: vec4<f32>,
    base_color: vec4<f32>,
    shadow_params: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> scene: Scene;

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) _normal: vec3<f32>) -> @builtin(position) vec4<f32> {
    return scene.light_view_proj * vec4<f32>(position, 1.0);
}
"#;

const TONEMAP_SHADER: &str = r#"
struct ToneMap {
    exposure: f32,
    gamma: f32,
    _pad0: f32,
    _pad1: f32,
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(hdr_tex));
    let texel = vec2<i32>(clamp(in.uv * dims, vec2<f32>(0.0), dims - vec2<f32>(1.0)));
    let hdr = textureLoad(hdr_tex, texel, 0).rgb * tone.exposure;
    let mapped = hdr / (hdr + vec3<f32>(1.0));
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

    tone_map_buffer: wgpu::Buffer,
    tone_map_bind_group_layout: wgpu::BindGroupLayout,
    tone_map_bind_group: wgpu::BindGroup,
    tone_map_pipeline: wgpu::RenderPipeline,

    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,

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
        scene_uniform.light_view_proj = compute_light_view_proj([
            scene_uniform.light_direction[0],
            scene_uniform.light_direction[1],
            scene_uniform.light_direction[2],
        ]);
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
                            view_dimension: wgpu::TextureViewDimension::D2,
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
            &shadow_target.view,
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
                buffers: &[Vertex::layout()],
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
                buffers: &[Vertex::layout()],
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

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube-vertex-buffer"),
            contents: bytemuck::cast_slice(&CUBE_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube-index-buffer"),
            contents: bytemuck::cast_slice(&CUBE_INDICES),
            usage: wgpu::BufferUsages::INDEX,
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
            tone_map_buffer,
            tone_map_bind_group_layout,
            tone_map_bind_group,
            tone_map_pipeline,
            vertex_buffer,
            index_buffer,
            index_count: CUBE_INDICES.len() as u32,
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

    pub fn update_camera(&mut self, view_proj: [[f32; 4]; 4], camera_position: [f32; 3]) {
        self.scene_uniform.view_proj = view_proj;
        self.scene_uniform.camera_position = [
            camera_position[0],
            camera_position[1],
            camera_position[2],
            1.0,
        ];
        self.scene_uniform.light_view_proj = compute_light_view_proj([
            self.scene_uniform.light_direction[0],
            self.scene_uniform.light_direction[1],
            self.scene_uniform.light_direction[2],
        ]);
        self.queue.write_buffer(
            &self.scene_buffer,
            0,
            bytemuck::bytes_of(&self.scene_uniform),
        );
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let swapchain_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("main-render-encoder"),
            });

        {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_target.view,
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
            shadow_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            shadow_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            shadow_pass.draw_indexed(0..self.index_count, 0, 0..1);
        }

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
            pbr_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pbr_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pbr_pass.draw_indexed(0..self.index_count, 0, 0..1);
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
        Ok(())
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
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        ShadowTarget {
            view,
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

fn compute_light_view_proj(light_direction: [f32; 3]) -> [[f32; 4]; 4] {
    let mut dir = Vec3::from_array(light_direction);
    if dir.length_squared() < 1e-6 {
        dir = Vec3::new(-0.5, -1.0, -0.3);
    }
    dir = dir.normalize();
    let center = Vec3::ZERO;
    let eye = center - dir * 8.0;
    let up = if dir.y.abs() > 0.98 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(eye, center, up);
    let proj = Mat4::orthographic_rh_gl(-6.0, 6.0, -6.0, 6.0, 0.1, 24.0);
    (proj * view).to_cols_array_2d()
}

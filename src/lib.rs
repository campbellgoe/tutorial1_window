mod texture;


use winit::{
  event::*,
  event_loop::{ControlFlow, EventLoop},
  window::WindowBuilder,
};

use wgpu::util::DeviceExt;
use cgmath::prelude::*;

#[cfg(target_arch="wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch="wasm32", wasm_bindgen(start))]
pub async fn run() {
  cfg_if::cfg_if! {
    if #[cfg(target_arch = "wasm32")] {
      std::panic::set_hook(Box::new(console_error_panic_hook::hook));
      console_log::init_with_level(log::Level::Warn).expect("Couldn't initialize logger");
    } else {
      env_logger::init();
    }
  }
  let event_loop = EventLoop::new();
  let window = WindowBuilder::new().build(&event_loop).unwrap();
  #[cfg(target_arch = "wasm32")]
  {
      // Winit prevents sizing with CSS, so we have to set
      // the size manually when on web.
      use winit::dpi::PhysicalSize;
      window.set_inner_size(PhysicalSize::new(450, 400));
      
      use winit::platform::web::WindowExtWebSys;
      web_sys::window()
          .and_then(|win| win.document())
          .and_then(|doc| {
              let dst = doc.get_element_by_id("wasm-example")?;
              let canvas = web_sys::Element::from(window.canvas());
              dst.append_child(&canvas).ok()?;
              Some(())
          })
          .expect("Couldn't append canvas to document body.");
  }
  let mut state = State::new(window).await;
  event_loop.run(move |event, _, control_flow| {
      match event {
          Event::WindowEvent {
              ref event,
              window_id,
          } if window_id == state.window().id() => if !state.input(event) {
              match event {
                  WindowEvent::CloseRequested
                  | WindowEvent::KeyboardInput {
                      input:
                          KeyboardInput {
                              state: ElementState::Pressed,
                              virtual_keycode: Some(VirtualKeyCode::Escape),
                              ..
                          },
                      ..
                  } => *control_flow = ControlFlow::Exit,
                  WindowEvent::Resized(physical_size) => {
                      state.resize(*physical_size);
                  }
                  WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                      // new_inner_size is &&mut so we have to dereference it twice
                      state.resize(**new_inner_size);
                  }
                  // other specific events you want to handle
                  _ => {} // catch-all for other events
              }
          }
          Event::RedrawRequested(window_id) if window_id == state.window().id() => {
            state.update();
            match state.render() {
                Ok(_) => {}
                // Reconfigure the surface if lost
                Err(wgpu::SurfaceError::Lost) => state.resize(state.size),
                // The system is out of memory, we should probably quit
                Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
                // All other errors (Outdated, Timeout) should be resolved by the next frame
                Err(e) => eprintln!("{:?}", e),
            }
          }
          Event::MainEventsCleared => {
              // RedrawRequested will only trigger once unless we manually
              // request it.
              state.window().request_redraw();
          }
          _ => {} // catch-all for non-window events
      }
  });
}
use winit::window::Window;

struct CameraController {
  speed: f32,
  is_forward_pressed: bool,
  is_backward_pressed: bool,
  is_left_pressed: bool,
  is_right_pressed: bool,
}

impl CameraController {
  fn new(speed: f32) -> Self {
      Self {
          speed,
          is_forward_pressed: false,
          is_backward_pressed: false,
          is_left_pressed: false,
          is_right_pressed: false,
      }
  }

  fn process_events(&mut self, event: &WindowEvent) -> bool {
      match event {
          WindowEvent::KeyboardInput {
              input: KeyboardInput {
                  state,
                  virtual_keycode: Some(keycode),
                  ..
              },
              ..
          } => {
              let is_pressed = *state == ElementState::Pressed;
              match keycode {
                  VirtualKeyCode::W | VirtualKeyCode::Up => {
                      self.is_forward_pressed = is_pressed;
                      true
                  }
                  VirtualKeyCode::A | VirtualKeyCode::Left => {
                      self.is_left_pressed = is_pressed;
                      true
                  }
                  VirtualKeyCode::S | VirtualKeyCode::Down => {
                      self.is_backward_pressed = is_pressed;
                      true
                  }
                  VirtualKeyCode::D | VirtualKeyCode::Right => {
                      self.is_right_pressed = is_pressed;
                      true
                  }
                  _ => false,
              }
          }
          _ => false,
      }
  }

  fn update_camera(&self, camera: &mut Camera) {
      use cgmath::InnerSpace;
      let forward = camera.target - camera.eye;
      let forward_norm = forward.normalize();
      let forward_mag = forward.magnitude();

      // Prevents glitching when the camera gets too close to the
      // center of the scene.
      if self.is_forward_pressed && forward_mag > self.speed {
          camera.eye += forward_norm * self.speed;
      }
      if self.is_backward_pressed {
          camera.eye -= forward_norm * self.speed;
      }

      let right = forward_norm.cross(camera.up);

      // Redo radius calc in case the forward/backward is pressed.
      let forward = camera.target - camera.eye;
      let forward_mag = forward.magnitude();

      if self.is_right_pressed {
          // Rescale the distance between the target and the eye so 
          // that it doesn't change. The eye, therefore, still 
          // lies on the circle made by the target and eye.
          camera.eye = camera.target - (forward + right * self.speed).normalize() * forward_mag;
      }
      if self.is_left_pressed {
          camera.eye = camera.target - (forward - right * self.speed).normalize() * forward_mag;
      }
  }
}

struct Instance {
  position: cgmath::Vector3<f32>,
  rotation: cgmath::Quaternion<f32>,
}

impl Instance {
  fn to_raw(&self) -> InstanceRaw {
      InstanceRaw {
          model: (cgmath::Matrix4::from_translation(self.position) * cgmath::Matrix4::from(self.rotation)).into(),
      }
  }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
}

impl InstanceRaw {
  fn desc() -> wgpu::VertexBufferLayout<'static> {
      use std::mem;
      wgpu::VertexBufferLayout {
          array_stride: mem::size_of::<InstanceRaw>() as wgpu::BufferAddress,
          // We need to switch from using a step mode of Vertex to Instance
          // This means that our shaders will only change to use the next
          // instance when the shader starts processing a new instance
          step_mode: wgpu::VertexStepMode::Instance,
          attributes: &[
              // A mat4 takes up 4 vertex slots as it is technically 4 vec4s. We need to define a slot
              // for each vec4. We'll have to reassemble the mat4 in the shader.
              wgpu::VertexAttribute {
                  offset: 0,
                  // While our vertex shader only uses locations 0, and 1 now, in later tutorials, we'll
                  // be using 2, 3, and 4, for Vertex. We'll start at slot 5, not conflict with them later
                  shader_location: 5,
                  format: wgpu::VertexFormat::Float32x4,
              },
              wgpu::VertexAttribute {
                  offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                  shader_location: 6,
                  format: wgpu::VertexFormat::Float32x4,
              },
              wgpu::VertexAttribute {
                  offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                  shader_location: 7,
                  format: wgpu::VertexFormat::Float32x4,
              },
              wgpu::VertexAttribute {
                  offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                  shader_location: 8,
                  format: wgpu::VertexFormat::Float32x4,
              },
          ],
      }
  }
}

// We need this for Rust to store our data correctly for the shaders
#[repr(C)]
// This is so we can store this in a buffer
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    // We can't use cgmath with bytemuck directly, so we'll have
    // to convert the Matrix4 into a 4x4 f32 array
    view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
    fn new() -> Self {
        use cgmath::SquareMatrix;
        Self {
            view_proj: cgmath::Matrix4::identity().into(),
        }
    }

    fn update_view_proj(&mut self, camera: &Camera) {
        self.view_proj = camera.build_view_projection_matrix().into();
    }
}
 
struct Camera {
    eye: cgmath::Point3<f32>,
    target: cgmath::Point3<f32>,
    up: cgmath::Vector3<f32>,
    aspect: f32,
    fovy: f32,
    znear: f32,
    zfar: f32,
}
#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.5,
    0.0, 0.0, 0.0, 1.0,
);
 
impl Camera {
    fn build_view_projection_matrix(&self) -> cgmath::Matrix4<f32> {
        // 1.
        let view = cgmath::Matrix4::look_at_rh(self.eye, self.
            target, self.up);
        // 2.
        let proj = cgmath::perspective(cgmath::Deg(self.fovy), self.aspect, self.znear, self.zfar);

        // 3.
        return OPENGL_TO_WGPU_MATRIX * proj * view;
    }
}
struct State {
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    // The window must be declared after the surface so
    // it gets dropped after it as the surface contains
    // unsafe references to the window's resources.
    window: Window,
    clear_color: wgpu::Color,
    render_pipeline: wgpu::RenderPipeline,
    // color_pipeline: wgpu::RenderPipeline,
    pipeline_toggle: bool,
    // vertex_buffer: wgpu::Buffer,
    // num_vertices: u32,
    // index_buffer: wgpu::Buffer, 
    // num_indices: u32,
    hexagon_vertex_buffer: wgpu::Buffer,
    hexagon_index_buffer: wgpu::Buffer,
    hexagon_num_indices: u32,
    square_vertex_buffer: wgpu::Buffer,
    square_index_buffer: wgpu::Buffer,
    square_num_indices: u32,
    spacebar_toggle: bool,
    diffuse_bind_group: wgpu::BindGroup,
    noise_bind_group: wgpu::BindGroup,
    diffuse_texture: texture::Texture,
    noise_texture: texture::Texture,
    camera: Camera,
    camera_uniform: CameraUniform,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_controller: CameraController,
    instances: Vec<Instance>,
    instance_buffer: wgpu::Buffer,
    depth_texture: texture::Texture,
}

impl State {
  fn get_texture_bind_group(&self) -> &wgpu::BindGroup {
    if self.spacebar_toggle {
        &self.diffuse_bind_group
    } else {
       &self.noise_bind_group
    }
}
// Creating some of the wgpu types requires async code
  async fn new(window: Window) -> Self {
    let size = window.inner_size();

    // The instance is a handle to our GPU
    // Backends::all => Vulkan + Metal + DX12 + Browser WebGPU
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });
    
    // # Safety
    //
    // The surface needs to live as long as the window that created it.
    // State owns the window, so this should be safe.
    let surface = unsafe { instance.create_surface(&window) }.unwrap();

    let adapter = instance.request_adapter(
        &wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        },
    ).await.unwrap();

    let (device, queue) = adapter.request_device(
    &wgpu::DeviceDescriptor {
        features: wgpu::Features::empty(),
        // WebGL doesn't support all of wgpu's features, so if
        // we're building for the web, we'll have to disable some.
        limits: if cfg!(target_arch = "wasm32") {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        },
        label: None,
    },
    None, // Trace path
    ).await.unwrap();
    
    let surface_caps = surface.get_capabilities(&adapter);
    // Shader code in this tutorial assumes an sRGB surface texture. Using a different
    // one will result in all the colors coming out darker. If you want to support non
    // sRGB surfaces, you'll need to account for that when drawing to the frame.
    let surface_format = surface_caps.formats.iter()
        .copied()
        .filter(|f| f.is_srgb())
        .next()
        .unwrap_or(surface_caps.formats[0]);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width,
        height: size.height,
        present_mode: surface_caps.present_modes[0],
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&device, &config);

    let diffuse_bytes = include_bytes!("happy-tree.png");
    let diffuse_texture = texture::Texture::from_bytes(&device, &queue, diffuse_bytes, "happy-tree.png").unwrap(); 

    let noise_bytes = include_bytes!("layered-simplex-noise.png");
    let noise_texture = texture::Texture::from_bytes(&device, &queue, noise_bytes, "layered-simplex-noise.png").unwrap(); 

    const NUM_INSTANCES_PER_ROW: u32 = 10;
    const INSTANCE_DISPLACEMENT: cgmath::Vector3<f32> = cgmath::Vector3::new(NUM_INSTANCES_PER_ROW as f32 * 0.5, 0.0, NUM_INSTANCES_PER_ROW as f32 * 0.5);

    let instances = (0..NUM_INSTANCES_PER_ROW).flat_map(|z| {
      (0..NUM_INSTANCES_PER_ROW).map(move |x| {
          let position = cgmath::Vector3 { x: x as f32, y: 0.0, z: z as f32 } - INSTANCE_DISPLACEMENT;

          let rotation = if position.is_zero() {
              // this is needed so an object at (0, 0, 0) won't get scaled to zero
              // as Quaternions can affect scale if they're not created correctly
              cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0))
          } else {
              cgmath::Quaternion::from_axis_angle(position.normalize(), cgmath::Deg(45.0))
          };

          Instance {
              position, rotation,
          }
      })
    }).collect::<Vec<_>>();

    let instance_data = instances.iter().map(Instance::to_raw).collect::<Vec<_>>();
    let instance_buffer = device.create_buffer_init(
      &wgpu::util::BufferInitDescriptor {
          label: Some("Instance Buffer"),
          contents: bytemuck::cast_slice(&instance_data),
          usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, // Include COPY_DST here
      }
    );
   
    let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
          entries: &[
              wgpu::BindGroupLayoutEntry {
                  binding: 0,
                  visibility: wgpu::ShaderStages::FRAGMENT,
                  ty: wgpu::BindingType::Texture {
                      multisampled: false,
                      view_dimension: wgpu::TextureViewDimension::D2,
                      sample_type: wgpu::TextureSampleType::Float { filterable: true },
                  },
                  count: None,
              },
              wgpu::BindGroupLayoutEntry {
                  binding: 1,
                  visibility: wgpu::ShaderStages::FRAGMENT,
                  // This should match the filterable field of the
                  // corresponding Texture entry above.
                  ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                  count: None,
              },
          ],
          label: Some("texture_bind_group_layout"),
      });
      let diffuse_bind_group = device.create_bind_group(
        &wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                }
            ],
            label: Some("diffuse_bind_group"),
        }
  );
       
  let noise_bind_group = device.create_bind_group(
    &wgpu::BindGroupDescriptor {
        layout: &texture_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&noise_texture.view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&noise_texture.sampler),
            }
        ],
        label: Some("noise_bind_group"),
    }
);
let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
  entries: &[
      wgpu::BindGroupLayoutEntry {
          binding: 0,
          visibility: wgpu::ShaderStages::VERTEX,
          ty: wgpu::BindingType::Buffer {
              ty: wgpu::BufferBindingType::Uniform,
              has_dynamic_offset: false,
              min_binding_size: None,
          },
          count: None,
      }
  ],
  label: Some("camera_bind_group_layout"),
});
      let depth_texture = texture::Texture::create_depth_texture(&device, &config, "depth_texture");
      let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
      let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
          label: Some("Render Pipeline Layout"),
          bind_group_layouts: &[
            &texture_bind_group_layout,
            &camera_bind_group_layout,
        ],
          push_constant_ranges: &[],
      });
      let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
      label: Some("Render Pipeline"),
      layout: Some(&render_pipeline_layout),
      vertex: wgpu::VertexState {
          module: &shader,
          entry_point: "vs_main", 
          buffers: &[Vertex::desc(), InstanceRaw::desc()],
      },
      fragment: Some(wgpu::FragmentState {
          module: &shader,
          entry_point: "fs_main",
          targets: &[Some(wgpu::ColorTargetState {
              format: config.format,
              blend: Some(wgpu::BlendState::REPLACE),
              write_mask: wgpu::ColorWrites::ALL,
          })],
      }),
      primitive: wgpu::PrimitiveState {
          topology: wgpu::PrimitiveTopology::TriangleList,
          strip_index_format: None,
          front_face: wgpu::FrontFace::Ccw,
          cull_mode: Some(wgpu::Face::Back),
          // Setting this to anything other than Fill requires Features::NON_FILL_POLYGON_MODE
          polygon_mode: wgpu::PolygonMode::Fill,
          // Requires Features::DEPTH_CLIP_CONTROL
          unclipped_depth: false,
          // Requires Features::CONSERVATIVE_RASTERIZATION
          conservative: false,
      },
      depth_stencil: Some(wgpu::DepthStencilState {
        format: texture::Texture::DEPTH_FORMAT,
        depth_write_enabled: true,
        depth_compare: wgpu::CompareFunction::Less,
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }),
      multisample: wgpu::MultisampleState {
          count: 1, 
          mask: !0,
          alpha_to_coverage_enabled: false,
      },
      multiview: None,
  });
  // let color_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
  //     label: Some("Color Pipeline"),
  //     layout: Some(&render_pipeline_layout), // You can reuse the layout
  //     vertex: wgpu::VertexState {
  //         module: &shader, // Reuse the shader module
  //         entry_point: "vs_main", // Entry point for the vertex shader
  //         buffers: &[
  //         Vertex::desc(),
  //         ],
  //     },
  //     fragment: Some(wgpu::FragmentState { // Fragment shader
  //         module: &shader, // Reuse the shader module
  //         entry_point: "fs_main", // Entry point for the fragment shader
  //         targets: &[Some(wgpu::ColorTargetState {
  //             format: config.format,
  //             blend: Some(wgpu::BlendState::REPLACE),
  //             write_mask: wgpu::ColorWrites::ALL,
  //         })],
  //     }),
  //     primitive: wgpu::PrimitiveState {
  //         topology: wgpu::PrimitiveTopology::TriangleList,
  //         strip_index_format: None,
  //         front_face: wgpu::FrontFace::Ccw,
  //         cull_mode: Some(wgpu::Face::Back),
  //         polygon_mode: wgpu::PolygonMode::Fill,
  //         unclipped_depth: false,
  //         conservative: false,
  //     },
  //     depth_stencil: None,
  //     multisample: wgpu::MultisampleState {
  //         count: 1,
  //         mask: !0,
  //         alpha_to_coverage_enabled: false,
  //     },
  //     multiview: None,
  // });
  // let vertex_buffer = device.create_buffer_init(
  //   &wgpu::util::BufferInitDescriptor {
  //       label: Some("Vertex Buffer"),
  //       contents: bytemuck::cast_slice(VERTICES),
  //       usage: wgpu::BufferUsages::VERTEX,
  //   }
  // );
  // let index_buffer = device.create_buffer_init(
  //   &wgpu::util::BufferInitDescriptor {
  //       label: Some("Index Buffer"),
  //       contents: bytemuck::cast_slice(INDICES),
  //       usage: wgpu::BufferUsages::INDEX,
  //   }
  // );
  // let num_indices = INDICES.len() as u32;
  // let num_vertices = VERTICES.len() as u32;

  let hexagon_vertex_buffer = device.create_buffer_init(
    &wgpu::util::BufferInitDescriptor {
        label: Some("Vertex Buffer"),
        contents: bytemuck::cast_slice(HEXAGON_VERTICES),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    }
    );
  let hexagon_index_buffer = device.create_buffer_init(
    &wgpu::util::BufferInitDescriptor {
        label: Some("Index Buffer"),
        contents: bytemuck::cast_slice(HEXAGON_INDICES),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
    }
    );
  let hexagon_num_indices = HEXAGON_INDICES.len() as u32;

  let square_vertex_buffer = device.create_buffer_init(
    &wgpu::util::BufferInitDescriptor {
        label: Some("Vertex Buffer"),
        contents: bytemuck::cast_slice(SQUARE_VERTICES),
        usage: wgpu::BufferUsages::VERTEX,
    }
    );
  let square_index_buffer = device.create_buffer_init(
    &wgpu::util::BufferInitDescriptor {
        label: Some("Index Buffer"),
        contents: bytemuck::cast_slice(SQUARE_INDICES),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
    }
    );
  let square_num_indices = SQUARE_INDICES.len() as u32;

  let camera = Camera {
    // position the camera 1 unit up and 2 units back
    // +z is out of the screen
    eye: (0.0, 1.0, 2.0).into(),
    // have it look at the origin
    target: (0.0, 0.0, 0.0).into(),
    // which way is "up"
    up: cgmath::Vector3::unit_y(),
    aspect: config.width as f32 / config.height as f32,
    fovy: 45.0,
    znear: 0.1,
    zfar: 100.0,
  };
  let mut camera_uniform = CameraUniform::new();
  camera_uniform.update_view_proj(&camera);
  
  let camera_buffer = device.create_buffer_init(
      &wgpu::util::BufferInitDescriptor {
          label: Some("Camera Buffer"),
          contents: bytemuck::cast_slice(&[camera_uniform]),
          usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      }
  );
  

  let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
    layout: &camera_bind_group_layout,
    entries: &[
        wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buffer.as_entire_binding(),
        }
    ],
    label: Some("camera_bind_group"),
  });
  let camera_controller = CameraController::new(0.2);
  Self {
    window,
    surface,
    device,
    queue,
    config,
    size,
    clear_color: wgpu::Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 },
    render_pipeline,
    // color_pipeline,
    pipeline_toggle: false,
//   vertex_buffer,
//   num_vertices,
//   index_buffer,
//   num_indices,
    hexagon_vertex_buffer,
    hexagon_index_buffer,
    hexagon_num_indices,
    square_vertex_buffer,
    square_index_buffer,
    square_num_indices,
    spacebar_toggle: false,
    diffuse_bind_group,
    noise_bind_group,
    diffuse_texture,
    noise_texture,
    camera,
    camera_uniform,
    camera_buffer,
    camera_bind_group,
    camera_controller,
    instances,
    instance_buffer,
    depth_texture,
  }
}

  pub fn window(&self) -> &Window {
      &self.window
  }

  pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
    if new_size.width > 0 && new_size.height > 0 {
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_texture = texture::Texture::create_depth_texture(&self.device, &self.config, "depth_texture");
    }
  }

  fn input(&mut self, event: &WindowEvent) -> bool {
    if self.camera_controller.process_events(event) {
      true
    } else {
      match event {
          WindowEvent::CursorMoved { position, .. } => {
              // Normalize the cursor position to 0.0 - 1.0
              let x = position.x as f64 / self.size.width as f64;
              let y = position.y as f64 / self.size.height as f64;

              // Update clear color based on the position
              self.clear_color = wgpu::Color {
                  r: x,
                  g: y,
                  b: (x+y)/2.0, // Example: fixed blue component
                  a: 1.0,
              };

              true
          }
          WindowEvent::KeyboardInput {
            input:
                KeyboardInput {
                    state: ElementState::Pressed,
                    virtual_keycode: Some(VirtualKeyCode::Space),
                    ..
                },
            ..
          } => {
              // Add logic to toggle between pipelines
              self.pipeline_toggle = !self.pipeline_toggle;
              self.spacebar_toggle = !self.spacebar_toggle;
              println!("Spacebar pressed, toggle value: {}", self.spacebar_toggle);
              true
          }
          _ => false,
      }
    }
  }

  fn update(&mut self) {
    let delta_rotation = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(1.0));

    for instance in self.instances.iter_mut() {
        // Rotate the instance around the Z-axis
        instance.rotation = instance.rotation * delta_rotation;
    }

    self.camera_controller.update_camera(&mut self.camera);
    self.camera_uniform.update_view_proj(&self.camera);
    self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&[self.camera_uniform]));
    let instance_data = self.instances.iter().map(Instance::to_raw).collect::<Vec<_>>();
    self.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instance_data));
  }

  fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
    let output = self.surface.get_current_texture()?;
    let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Render Encoder"),
    });
    {
      let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
          label: Some("Render Pass"),
          color_attachments: &[Some(wgpu::RenderPassColorAttachment {
              view: &view,
              resolve_target: None,
              ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(self.clear_color),
                store: wgpu::StoreOp::Store,
              },
          })],
          depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &self.depth_texture.view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
          }),
          occlusion_query_set: None,
          timestamp_writes: None,
      });

      let pipeline = &self.render_pipeline;/*if self.spacebar_toggle {
        &self.color_pipeline
      } else {
        &self.render_pipeline
      };*/

      let (vertex_buffer, index_buffer, num_indices) = if self.spacebar_toggle {
        (&self.hexagon_vertex_buffer, &self.hexagon_index_buffer, self.hexagon_num_indices)
      } else {
        (&self.square_vertex_buffer, &self.square_index_buffer, self.square_num_indices)
      };

      let current_bind_group = self.get_texture_bind_group();

      // Use the selected pipeline
      render_pass.set_pipeline(pipeline);
      render_pass.set_bind_group(0, current_bind_group, &[]);
      render_pass.set_bind_group(1, &self.camera_bind_group, &[]);
      render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
      render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
      render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
      render_pass.draw_indexed(0..num_indices, 0, 0..self.instances.len() as _);
      //render_pass.draw(0..self.num_vertices, 0..1);
    }

    // submit will accept anything that implements IntoIter
    self.queue.submit(std::iter::once(encoder.finish()));
    output.present();

    Ok(())
  }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

const HEXAGON_VERTICES: &[Vertex] = &[
    Vertex { position: [ 0.0,  0.0, 0.0], tex_coords: [0.5, 0.5], }, // Center
    Vertex { position: [ 0.0,  1.0, 0.0], tex_coords: [1.0, 0.5] }, // Top
    Vertex { position: [-0.86,  0.5, 0.0], tex_coords: [0.75, 0.9330127018922193] }, // Top Right
    Vertex { position: [-0.86, -0.5, 0.0], tex_coords: [0.25, 0.9330127018922194] }, // Bottom Right
    Vertex { position: [ 0.0, -1.0, 0.0], tex_coords: [0.0, 0.5] }, // Bottom
    Vertex { position: [ 0.86, -0.5, 0.0], tex_coords: [0.25, 0.06698729810778081] }, // Bottom Left
    Vertex { position: [ 0.86,  0.5, 0.0], tex_coords: [0.75, 0.06698729810778048] }, // Top Left
];

const HEXAGON_INDICES: &[u16] = &[
    0, 1, 2,
    0, 2, 3,
    0, 3, 4,
    0, 4, 5,
    0, 5, 6,
    0, 6, 1,
];

const SQUARE_VERTICES: &[Vertex] = &[
    Vertex { position: [-1.0,  1.0, 0.0], tex_coords: [0.0, 1.0], }, // Top-left
    Vertex { position: [ 1.0,  1.0, 0.0], tex_coords: [1.0, 1.0], }, // Top-right
    Vertex { position: [-1.0, -1.0, 0.0], tex_coords: [0.0, 0.0], }, // Bottom-left
    Vertex { position: [ 1.0, -1.0, 0.0], tex_coords: [1.0, 0.0], }, // Bottom-right
];

const SQUARE_INDICES: &[u16] = &[
    0, 2, 3, // First triangle: top-left, bottom-left, bottom-right
    0, 3, 1, // Second triangle: top-left, bottom-right, top-right
];

impl Vertex {
  fn desc() -> wgpu::VertexBufferLayout<'static> {
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
              }
          ]
      }
  }
}

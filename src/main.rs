#[macro_use]
extern crate glium;

use std::ffi::CString;
use std::mem;
use std::ops::Add;
use std::time::{Duration, Instant};

use clap::{App, Arg};
use glium::glutin;
use glium::glutin::dpi::{PhysicalPosition, PhysicalSize, Position};
use glium::glutin::event::{Event, WindowEvent};
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::glutin::platform::unix::x11;
use glium::glutin::platform::unix::{
    EventLoopWindowTargetExtUnix, WindowBuilderExtUnix, WindowExtUnix,
};
use glium::vertex::VertexBufferAny;
use glium::Surface;
use x11cap::{Bgr8, CaptureSource, Capturer};

struct Settings {
    window_title: String,
    target_fps: u32,
    offscreen: bool,
}

fn main() {
    let matches = App::new("Screen splitter")
        .version("0.1")
        .about("Allows the user to share a single monitor in a video call")
        .arg(
            Arg::with_name("monitor-id")
                .index(1)
                .help("The ID of the monitor to mirror")
                .default_value("1")
                .required(true)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("fps")
                .long("fps")
                .help("Target frames per second")
                .default_value("30")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("onscreen")
                .long("onscreen")
                .help("Show the capture window on screen")
                .takes_value(false),
        )
        .get_matches();

    let monitor_id = match matches.value_of("monitor-id").unwrap().parse::<usize>() {
        Ok(parsed_id) => parsed_id,
        Err(_) => {
            eprintln!("Monitor ID must be an integer");
            return;
        }
    };

    let target_fps = match matches.value_of("fps").unwrap().parse::<u32>() {
        Ok(parsed_id) => parsed_id,
        Err(_) => {
            eprintln!("Target frames per second must be integer");
            return;
        }
    };

    let onscreen = matches.is_present("onscreen");

    let source: CaptureSource = CaptureSource::Monitor(monitor_id);

    display_capture_window(
        Settings {
            window_title: format!("Monitor {}", monitor_id),
            target_fps,
            offscreen: !onscreen,
        },
        source,
    );
}

/// Create a window and mirror the image of the capture source
fn display_capture_window(config: Settings, source: CaptureSource) {
    let mut capturer = Capturer::new(source).expect("Unable to create screen capturer");
    let geo = capturer.get_geometry();
    let target_duration = Duration::new(0, 1_000_000_000u32 / config.target_fps);

    let el = glutin::event_loop::EventLoop::new();
    let display = create_offscreen_window(&el, config, geo.width as i32, geo.height as i32);

    #[derive(Copy, Clone)]
    struct Vertex {
        position: [f32; 2],
    }

    implement_vertex!(Vertex, position);

    let vb: VertexBufferAny = glium::VertexBuffer::new(
        &display,
        &[
            Vertex {
                position: [-1.0, 1.0],
            },
            Vertex {
                position: [1.0, 1.0],
            },
            Vertex {
                position: [-1.0, -1.0],
            },
            Vertex {
                position: [1.0, -1.0],
            },
        ],
    )
    .unwrap()
    .into();

    let ib = glium::index::NoIndices(glium::index::PrimitiveType::TriangleStrip);

    let program = glium::Program::from_source(
        &display,
        // Vertex shader
        //
        // We use the vertex shader to flip the image which would otherwise be upside down
        r"
                #version 330

                in vec2 position;
                out vec2 v_tex_coords;

                void main() {
                    v_tex_coords = position * vec2(0.5, -0.5) + vec2(0.5);
                    gl_Position = vec4(position, 0.0, 1.0);
                }
        ",
        // Fragment shader
        //
        // Since the image we get from X11 is BGR and we need RGB (blue and red are flipped)
        // we correct this in the fragment shader. Doing this on the CPU would take too long
        r"
                #version 330

                in vec2 v_tex_coords;
                uniform sampler2D tex;

                void main() {
                    vec4 textureColor = texture(tex, v_tex_coords);
                    gl_FragColor = vec4(textureColor.b, textureColor.g, textureColor.r, 1);
                }
        ",
        None,
    )
    .expect("Error compiling shaders");

    let mut next_iteration = Instant::now();
    el.run(move |event, _, control_flow| {
        let early_wakeup = next_iteration > Instant::now();

        match event {
            Event::LoopDestroyed => return,
            Event::NewEvents(_) if !early_wakeup => {
                let start_time = Instant::now();

                // Capture the screen
                let captured_frame = capturer.capture_frame().expect("Failed to capture frame");
                let (width, height) = captured_frame.get_dimensions();
                let pixel_data = unsafe {
                    let slice = captured_frame.as_slice();
                    std::slice::from_raw_parts(
                        slice.as_ptr() as *const u8,
                        slice.len() * mem::size_of::<Bgr8>(),
                    )
                };

                // Create a texture containing the image data
                let data =
                    glium::texture::RawImage2d::from_raw_rgba(pixel_data.to_vec(), (width, height));
                let dest_texture =
                    glium::texture::srgb_texture2d::SrgbTexture2d::new(&display, data)
                        .expect("Unable to create texture");

                // Draw and display the frame
                let mut target = display.draw();
                let uniforms = uniform! { tex: &dest_texture };
                target
                    .draw(&vb, &ib, &program, &uniforms, &Default::default())
                    .expect("Unable to execute shader");
                target.finish().expect("Buffer swap failed");

                // Calculate the tome of the next wakeup
                let duration = start_time.elapsed();
                next_iteration = if target_duration >= duration {
                    let time_to_next_draw = target_duration - duration;
                    Instant::now().add(time_to_next_draw)
                } else {
                    Instant::now()
                };
                *control_flow = ControlFlow::WaitUntil(next_iteration);
            }
            Event::NewEvents(_) if early_wakeup => {
                // Wait again if there was an early wakeup
                *control_flow = ControlFlow::WaitUntil(next_iteration);
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                _ => (),
            },
            _ => (),
        }
    });
}

/// This function creates a off screen window.
///
/// As it turns out this is **A LOT** harder than you might imagine. Usually window managers ignore
/// requests to place windows outside the screen area due to the assumption that the request was
/// made in error. However, we actually really want the window to be not visible by the user.
///
/// To achieve this we set the override-redirect option when creating the window. This option
/// means that the window is not managed by the window manager. It also has the side effect that
/// the window is not decorated and no task bar entry is created. However, it also causes the
/// WM_STATE property not to be set by the window manager.
///
/// Since chrome uses the WM_STATE property to determine which windows are displayed in the window
/// selection dialog we need to set this property manually. Setting this property turned out to be
/// really hard, but thankfully winit allows us to do this after jumping through some hoops.
fn create_offscreen_window(
    el: &EventLoop<()>,
    config: Settings,
    width: i32,
    height: i32,
) -> glium::Display {
    // Build a new window. Make sure to set the override_redirect option so the window is not
    // managed by the window manager.
    let wb = glutin::window::WindowBuilder::new()
        .with_title(config.window_title)
        .with_inner_size(PhysicalSize::new(width, height))
        .with_override_redirect(config.offscreen);

    let cb = glutin::ContextBuilder::new();
    let display = glium::Display::new(wb, cb, &el).unwrap();

    {
        let gl_window = display.gl_window();
        let window = gl_window.window();

        if config.offscreen {
            // Move the window outside the visible screen area
            window.set_outer_position(Position::Physical(PhysicalPosition::new(
                width * -1,
                height * -1,
            )));
        }

        // Set the WM_STATE property so the window is shown in the chrome window selection dialog
        let xlib = el.xlib_xconnection().unwrap();
        let window_id = window.xlib_window().unwrap();
        let wm_state_atom = xlib.get_atom(CString::new("WM_STATE").unwrap().as_c_str());
        xlib.change_property(
            window_id,
            wm_state_atom,
            wm_state_atom,
            x11::util::PropMode::Replace,
            &[
                1 as x11::util::Cardinal, // NormalState
                0 as x11::util::Cardinal, // None
            ],
        )
        .flush()
        .unwrap();
    }

    return display;
}

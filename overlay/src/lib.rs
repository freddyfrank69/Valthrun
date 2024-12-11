#![feature(str_from_utf16_endian)]
use std::time::Instant;

use clipboard::ClipboardSupport;
use copypasta::ClipboardContext;
use font::FontAtlasBuilder;
use imgui::{
    Context,
    FontAtlas,
};
use imgui_winit_support::{
    winit::{
        event::{
            Event,
            WindowEvent,
        },
        event_loop::{
            ControlFlow,
            EventLoop,
        },
        platform::{
            run_return::EventLoopExtRunReturn,
            windows::WindowExtWindows,
        },
        window::{
            Window,
            WindowBuilder,
        },
    },
    HiDpiMode,
    WinitPlatform,
};
use input::{
    KeyboardInputSystem,
    MouseInputSystem,
};
use obfstr::obfstr;
use vulkan::VulkanRenderBackend;
use window_tracker::{
    ActiveTracker,
    WindowTracker,
};
use windows::Win32::{
    Foundation::{
        BOOL,
        HWND,
    },
    Graphics::{
        Dwm::{
            DwmEnableBlurBehindWindow,
            DwmIsCompositionEnabled,
            DWM_BB_BLURREGION,
            DWM_BB_ENABLE,
            DWM_BLURBEHIND,
        },
        Gdi::{
            CreateRectRgn,
            DeleteObject,
        },
    },
    UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity,
        SetWindowLongA,
        SetWindowLongPtrA,
        SetWindowPos,
        ShowWindow,
        GWL_EXSTYLE,
        GWL_STYLE,
        HWND_TOPMOST,
        SWP_NOACTIVATE,
        SWP_NOMOVE,
        SWP_NOSIZE,
        SW_SHOWNOACTIVATE,
        WDA_EXCLUDEFROMCAPTURE,
        WDA_NONE,
        WS_CLIPSIBLINGS,
        WS_EX_LAYERED,
        WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT,
        WS_POPUP,
        WS_VISIBLE,
    },
};

mod clipboard;
mod error;
pub use error::*;
mod input;
mod window_tracker;
pub use window_tracker::OverlayTarget;

mod vulkan;

mod perf;
pub use perf::PerfTracker;

mod font;
mod util;

pub use font::UnicodeTextRenderer;
pub use util::show_error_message;

fn create_window(event_loop: &EventLoop<()>, title: &str) -> Result<Window> {
    let window = WindowBuilder::new()
        .with_title(title.to_owned())
        .with_visible(false)
        .build(&event_loop)?;

    {
        let hwnd = HWND(window.hwnd());
        unsafe {
            // Make it transparent
            SetWindowLongA(
                hwnd,
                GWL_STYLE,
                (WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS).0 as i32,
            );
            SetWindowLongPtrA(
                hwnd,
                GWL_EXSTYLE,
                (WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT).0
                    as isize,
            );

            if !DwmIsCompositionEnabled()?.as_bool() {
                return Err(OverlayError::DwmCompositionDisabled);
            }

            let mut bb: DWM_BLURBEHIND = Default::default();
            bb.dwFlags = DWM_BB_ENABLE | DWM_BB_BLURREGION;
            bb.fEnable = BOOL::from(true);
            bb.hRgnBlur = CreateRectRgn(0, 0, 1, 1);
            DwmEnableBlurBehindWindow(hwnd, &bb)?;
            DeleteObject(bb.hRgnBlur);

            // Move the window to the top
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    Ok(window)
}

fn create_imgui_context(_options: &OverlayOptions) -> Result<(WinitPlatform, imgui::Context)> {
    let mut imgui = Context::create();
    imgui.set_ini_filename(None);

    let platform = WinitPlatform::init(&mut imgui);

    match ClipboardContext::new() {
        Ok(backend) => imgui.set_clipboard_backend(ClipboardSupport(backend)),
        Err(error) => log::warn!("Failed to initialize clipboard: {}", error),
    };

    Ok((platform, imgui))
}

pub struct OverlayOptions {
    pub title: String,
    pub target: OverlayTarget,
    pub register_fonts_callback: Option<Box<dyn Fn(&mut FontAtlas) -> ()>>,
}

pub trait RenderBackend {
    fn update_fonts_texture(&mut self, imgui: &mut imgui::Context);
    fn render_frame(
        &mut self,
        perf: &mut PerfTracker,
        window: &Window,
        draw_data: &imgui::DrawData,
    );
}

pub struct System {
    pub event_loop: EventLoop<()>,

    pub window: Window,
    pub platform: WinitPlatform,

    pub imgui: Context,
    pub imgui_fonts: FontAtlasBuilder,
    pub imgui_register_fonts_callback: Option<Box<dyn Fn(&mut FontAtlas) -> ()>>,

    pub window_tracker: WindowTracker,

    renderer: Box<dyn RenderBackend>,
}

pub fn init(options: OverlayOptions) -> Result<System> {
    let window_tracker = WindowTracker::new(&options.target)?;

    let event_loop = EventLoop::new();
    let window = create_window(&event_loop, &options.title)?;

    let (mut platform, mut imgui) = create_imgui_context(&options)?;
    platform.attach_window(imgui.io_mut(), &window, HiDpiMode::Default);

    let mut imgui_fonts = FontAtlasBuilder::new();
    imgui_fonts.register_font(include_bytes!("../resources/Roboto-Regular.ttf"))?;
    imgui_fonts.register_font(include_bytes!("../resources/NotoSansTC-Regular.ttf"))?;
    /* fallback if we do not have the roboto version of the glyph */
    imgui_fonts.register_font(include_bytes!("../resources/unifont-15.1.05.otf"))?;
    imgui_fonts.register_codepoints(1..255);

    let renderer = Box::new(VulkanRenderBackend::new(&window, &mut imgui)?);
    Ok(System {
        event_loop,
        window,

        imgui,
        imgui_fonts,
        imgui_register_fonts_callback: options.register_fonts_callback,

        platform,
        window_tracker,

        renderer,
    })
}

const PERF_RECORDS: usize = 2048;

impl System {
    pub fn main_loop<U, R>(self, mut update: U, mut render: R) -> i32
    where
        U: FnMut(&mut SystemRuntimeController) -> bool + 'static,
        R: FnMut(&imgui::Ui, &UnicodeTextRenderer) -> bool + 'static,
    {
        let System {
            mut event_loop,
            window,

            imgui,
            imgui_fonts,
            imgui_register_fonts_callback,

            mut platform,
            window_tracker,

            mut renderer,
            ..
        } = self;

        let mut last_frame = Instant::now();

        let mut runtime_controller = SystemRuntimeController {
            hwnd: HWND(window.hwnd() as isize),

            imgui,
            imgui_fonts,

            active_tracker: ActiveTracker::new(),
            key_input_system: KeyboardInputSystem::new(),
            mouse_input_system: MouseInputSystem::new(),
            window_tracker,

            frame_count: 0,
            debug_overlay_shown: false,
        };

        let mut perf = PerfTracker::new(PERF_RECORDS);
        let result = event_loop.run_return(move |event, _, control_flow| {
            *control_flow = ControlFlow::Poll;
            platform.handle_event(runtime_controller.imgui.io_mut(), &window, &event);

            match event {
                // New frame
                Event::NewEvents(_) => {
                    perf.begin();
                    let now = Instant::now();
                    runtime_controller
                        .imgui
                        .io_mut()
                        .update_delta_time(now - last_frame);
                    last_frame = now;
                }

                // End of event processing
                Event::MainEventsCleared => {
                    perf.mark("events cleared");

                    /* Update */
                    {
                        if !runtime_controller.update_state(&window) {
                            *control_flow = ControlFlow::Exit;
                            return;
                        }

                        if !update(&mut runtime_controller) {
                            *control_flow = ControlFlow::Exit;
                            return;
                        }

                        if runtime_controller.imgui_fonts.fetch_reset_flag_updated() {
                            let font_atlas = runtime_controller.imgui.fonts();
                            font_atlas.clear();

                            let (font_sources, _glyph_memory) =
                                runtime_controller.imgui_fonts.build_font_source(18.0);

                            font_atlas.add_font(&font_sources);
                            if let Some(user_callback) = &imgui_register_fonts_callback {
                                user_callback(font_atlas);
                            }

                            renderer.update_fonts_texture(&mut runtime_controller.imgui);
                        }

                        perf.mark("update");
                    }

                    /* Generate frame */
                    let draw_data = {
                        if let Err(error) =
                            platform.prepare_frame(runtime_controller.imgui.io_mut(), &window)
                        {
                            *control_flow = ControlFlow::ExitWithCode(1);
                            log::error!("Platform implementation prepare_frame failed: {}", error);
                            return;
                        }

                        let ui = runtime_controller.imgui.frame();
                        let unicode_text =
                            UnicodeTextRenderer::new(ui, &mut runtime_controller.imgui_fonts);

                        let run = render(ui, &unicode_text);
                        if !run {
                            *control_flow = ControlFlow::ExitWithCode(0);
                            return;
                        }
                        if runtime_controller.debug_overlay_shown {
                            ui.window("Render Debug")
                                .position([200.0, 200.0], imgui::Condition::FirstUseEver)
                                .size([400.0, 400.0], imgui::Condition::FirstUseEver)
                                .build(|| {
                                    ui.text(format!("FPS: {: >4.2}", ui.io().framerate));
                                    ui.same_line_with_pos(100.0);

                                    ui.text(format!(
                                        "Frame Time: {:.2}ms",
                                        ui.io().delta_time * 1000.0
                                    ));
                                    ui.same_line_with_pos(275.0);

                                    ui.text("History length:");
                                    ui.same_line();
                                    let mut history_length = perf.history_length();
                                    ui.set_next_item_width(75.0);
                                    if ui
                                        .input_scalar("##history_length", &mut history_length)
                                        .build()
                                    {
                                        perf.set_history_length(history_length);
                                    }
                                    perf.render(ui, ui.content_region_avail());
                                });
                        }
                        perf.mark("render frame");

                        platform.prepare_render(ui, &window);
                        runtime_controller.imgui.render()
                    };

                    /* render */
                    renderer.render_frame(&mut perf, &window, draw_data);

                    runtime_controller.frame_rendered();
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => *control_flow = ControlFlow::Exit,
                _ => {}
            }
        });

        result
    }
}

pub struct SystemRuntimeController {
    pub hwnd: HWND,

    pub imgui: imgui::Context,
    pub imgui_fonts: FontAtlasBuilder,

    debug_overlay_shown: bool,

    active_tracker: ActiveTracker,
    mouse_input_system: MouseInputSystem,
    key_input_system: KeyboardInputSystem,

    window_tracker: WindowTracker,

    frame_count: u64,
}

impl SystemRuntimeController {
    fn update_state(&mut self, window: &Window) -> bool {
        self.mouse_input_system.update(window, self.imgui.io_mut());
        self.key_input_system.update(window, self.imgui.io_mut());
        self.active_tracker.update(window, self.imgui.io());
        if !self.window_tracker.update(window) {
            log::info!("Target window has been closed. Exiting overlay.");
            return false;
        }

        true
    }

    fn frame_rendered(&mut self) {
        self.frame_count += 1;
        if self.frame_count == 1 {
            /* initial frame */
            unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };

            self.window_tracker.mark_force_update();
        }
    }

    pub fn toggle_screen_capture_visibility(&self, should_be_visible: bool) {
        unsafe {
            let (target_state, state_name) = if should_be_visible {
                (WDA_NONE, "normal")
            } else {
                (WDA_EXCLUDEFROMCAPTURE, "exclude from capture")
            };

            if !SetWindowDisplayAffinity(self.hwnd, target_state).as_bool() {
                log::warn!(
                    "{} '{}'.",
                    obfstr!("Failed to change overlay display affinity to"),
                    state_name
                );
            }
        }
    }

    pub fn toggle_debug_overlay(&mut self, visible: bool) {
        self.debug_overlay_shown = visible;
    }

    pub fn debug_overlay_shown(&self) -> bool {
        self.debug_overlay_shown
    }
}

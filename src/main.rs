#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app;
mod model;
mod scanner;
mod treemap;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("LumaDisk")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 620.0])
            .with_app_id("com.lumadisk.app")
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "LumaDisk",
        options,
        Box::new(|context| Ok(Box::new(app::LumaDiskApp::new(context)))),
    )
}

fn app_icon() -> eframe::egui::IconData {
    let width = 64;
    let height = 64;
    let mut rgba = vec![0_u8; width * height * 4];
    let tiles = [
        (7, 8, 31, 27, [44, 178, 140, 255]),
        (35, 8, 22, 27, [112, 91, 218, 255]),
        (7, 39, 19, 18, [218, 139, 55, 255]),
        (30, 39, 27, 18, [47, 147, 201, 255]),
    ];
    for y in 0..height {
        for x in 0..width {
            let pixel = (y * width + x) * 4;
            rgba[pixel..pixel + 4].copy_from_slice(&[10, 14, 20, 255]);
            for (left, top, tile_width, tile_height, color) in tiles {
                if x >= left && x < left + tile_width && y >= top && y < top + tile_height {
                    rgba[pixel..pixel + 4].copy_from_slice(&color);
                }
            }
        }
    }
    eframe::egui::IconData {
        rgba,
        width: width as u32,
        height: height as u32,
    }
}

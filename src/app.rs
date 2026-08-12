use crate::{
    model::ScanResult,
    scanner::{self, ScanEvent},
    treemap,
};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Vec2,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BG: Color32 = Color32::from_rgb(9, 12, 17);
const PANEL: Color32 = Color32::from_rgb(15, 19, 26);
const PANEL_2: Color32 = Color32::from_rgb(20, 25, 34);
const BORDER: Color32 = Color32::from_rgb(39, 47, 61);
const TEXT: Color32 = Color32::from_rgb(235, 239, 246);
const MUTED: Color32 = Color32::from_rgb(137, 147, 165);
const ACCENT: Color32 = Color32::from_rgb(81, 229, 177);
const DANGER: Color32 = Color32::from_rgb(255, 102, 119);

#[derive(Clone, Copy, PartialEq)]
enum View {
    Heatmap,
    Largest,
    Duplicates,
}

#[derive(Clone, Copy, PartialEq)]
enum ColorMode {
    Type,
    Age,
}

#[derive(Default)]
struct Progress {
    files: usize,
    bytes: u64,
    current: PathBuf,
    phase: &'static str,
}

#[derive(Clone)]
enum DeleteTarget {
    File(PathBuf),
    Category {
        extension: String,
        paths: Vec<PathBuf>,
        total_bytes: u64,
    },
}

struct TrashOutcome {
    paths: Vec<PathBuf>,
    result: Result<(), String>,
}

pub struct LumaDiskApp {
    root_input: String,
    include_hidden: bool,
    find_duplicates: bool,
    scanning: bool,
    progress: Progress,
    receiver: Option<Receiver<ScanEvent>>,
    result: Option<ScanResult>,
    search: String,
    extension_filter: String,
    min_size_index: usize,
    age_index: usize,
    view: View,
    color_mode: ColorMode,
    selected: Option<usize>,
    selected_category: Option<String>,
    kept: HashSet<PathBuf>,
    confirm_delete: Option<DeleteTarget>,
    trash_receiver: Option<Receiver<TrashOutcome>>,
    notice: Option<(String, bool, Instant)>,
}

impl LumaDiskApp {
    pub fn new(context: &eframe::CreationContext<'_>) -> Self {
        configure_style(&context.egui_ctx);
        let root_input = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
            .display()
            .to_string();
        Self {
            root_input,
            include_hidden: false,
            find_duplicates: true,
            scanning: false,
            progress: Progress::default(),
            receiver: None,
            result: None,
            search: String::new(),
            extension_filter: "All types".to_owned(),
            min_size_index: 0,
            age_index: 0,
            view: View::Heatmap,
            color_mode: ColorMode::Type,
            selected: None,
            selected_category: None,
            kept: HashSet::new(),
            confirm_delete: None,
            trash_receiver: None,
            notice: None,
        }
    }

    fn begin_scan(&mut self) {
        if self.scanning || self.trash_receiver.is_some() {
            return;
        }
        let root = PathBuf::from(self.root_input.trim());
        if !root.is_dir() {
            self.notify("That folder does not exist or cannot be read.", true);
            return;
        }
        let (sender, receiver) = mpsc::channel();
        scanner::start_scan(root, self.include_hidden, self.find_duplicates, sender);
        self.receiver = Some(receiver);
        self.scanning = true;
        self.progress = Progress {
            phase: "Starting scan",
            ..Default::default()
        };
        self.selected = None;
        self.selected_category = None;
        self.result = None;
    }

    fn poll_scan(&mut self) {
        let events: Vec<ScanEvent> = self
            .receiver
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                ScanEvent::Progress {
                    files,
                    bytes,
                    current,
                    phase,
                } => {
                    self.progress = Progress {
                        files,
                        bytes,
                        current,
                        phase,
                    };
                }
                ScanEvent::Complete(result) => {
                    self.scanning = false;
                    self.receiver = None;
                    match result {
                        Ok(result) => {
                            let message = format!(
                                "Mapped {} files in {:.2}s",
                                format_count(result.files.len()),
                                result.elapsed_ms as f64 / 1000.0
                            );
                            self.result = Some(result);
                            self.notify(message, false);
                        }
                        Err(error) => self.notify(error, true),
                    }
                }
            }
        }
    }

    fn notify(&mut self, message: impl Into<String>, is_error: bool) {
        self.notice = Some((message.into(), is_error, Instant::now()));
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let Some(result) = &self.result else {
            return Vec::new();
        };
        let query = self.search.trim().to_ascii_lowercase();
        let min_size =
            [0, 1024 * 1024, 100 * 1024 * 1024, 1024 * 1024 * 1024][self.min_size_index] as u64;
        let now = unix_now();
        let max_age = [i64::MAX, 30 * 86_400, 365 * 86_400, 3 * 365 * 86_400][self.age_index];
        result
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                (self.extension_filter == "All types" || file.extension == self.extension_filter)
                    && file.size >= min_size
                    && (max_age == i64::MAX
                        || file.modified_unix > 0
                            && now.saturating_sub(file.modified_unix) <= max_age)
                    && (query.is_empty()
                        || file.name.to_ascii_lowercase().contains(&query)
                        || file
                            .path
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn remove_file_from_result(&mut self, path: &Path) {
        let Some(result) = self.result.as_mut() else {
            return;
        };
        if result.remove_file(path) {
            self.selected = None;
        }
    }

    fn start_trash(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() || self.trash_receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = trash::delete_all(&paths).map_err(|error| error.to_string());
            let _ = sender.send(TrashOutcome { paths, result });
        });
        self.trash_receiver = Some(receiver);
        self.notify("Moving selected files to Trash / Recycle Bin...", false);
    }

    fn poll_trash(&mut self) {
        let outcome = self
            .trash_receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok());
        let Some(outcome) = outcome else { return };
        self.trash_receiver = None;

        let moved_paths: Vec<PathBuf> = outcome
            .paths
            .iter()
            .filter(|path| outcome.result.is_ok() || !path.exists())
            .cloned()
            .collect();
        for path in &moved_paths {
            self.kept.remove(path);
            self.remove_file_from_result(path);
        }

        match outcome.result {
            Ok(()) => self.notify(
                format!(
                    "Moved {} file{} to Trash / Recycle Bin.",
                    moved_paths.len(),
                    if moved_paths.len() == 1 { "" } else { "s" }
                ),
                false,
            ),
            Err(error) => self.notify(
                format!(
                    "Trash operation stopped: {error}. {} file{} moved successfully.",
                    moved_paths.len(),
                    if moved_paths.len() == 1 { "" } else { "s" }
                ),
                true,
            ),
        }
        let category_still_exists = self.selected_category.as_ref().is_some_and(|extension| {
            self.result
                .as_ref()
                .is_some_and(|result| result.files.iter().any(|file| &file.extension == extension))
        });
        if !category_still_exists {
            self.selected_category = None;
        }
    }

    fn reveal(&mut self, index: usize) {
        let Some(path) = self
            .result
            .as_ref()
            .and_then(|result| result.files.get(index))
            .map(|file| file.path.clone())
        else {
            return;
        };
        showfile::show_path_in_file_manager(&path);
    }

    fn move_selected(&mut self, index: usize) {
        let Some(source) = self
            .result
            .as_ref()
            .and_then(|result| result.files.get(index))
            .map(|file| file.path.clone())
        else {
            return;
        };
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Move file to...")
            .pick_folder()
        else {
            return;
        };
        let Some(file_name) = source.file_name() else {
            return;
        };
        let destination = folder.join(file_name);
        if destination.exists() {
            self.notify(
                "A file with that name already exists in the destination.",
                true,
            );
            return;
        }
        let moved = std::fs::rename(&source, &destination).or_else(|_| {
            std::fs::copy(&source, &destination)?;
            std::fs::remove_file(&source)
        });
        match moved {
            Ok(()) => {
                self.remove_file_from_result(&source);
                self.notify(format!("Moved to {}", destination.display()), false);
            }
            Err(error) => self.notify(format!("Move failed: {error}"), true),
        }
    }

    fn header(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(22, 14)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LUMA").size(20.0).strong().color(ACCENT));
                    ui.label(RichText::new("DISK").size(20.0).strong().color(TEXT));
                    ui.add_space(18.0);
                    ui.label(RichText::new("See what is taking your space.").color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let scan_text = if self.scanning {
                            "Scanning..."
                        } else if self.trash_receiver.is_some() {
                            "Moving..."
                        } else {
                            "Scan folder"
                        };
                        if ui
                            .add_enabled(
                                !self.scanning && self.trash_receiver.is_none(),
                                egui::Button::new(RichText::new(scan_text).strong().color(BG))
                                    .fill(ACCENT)
                                    .corner_radius(8)
                                    .min_size(Vec2::new(112.0, 34.0)),
                            )
                            .clicked()
                        {
                            self.begin_scan();
                        }
                        if ui
                            .add(egui::Button::new("Browse").min_size(Vec2::new(76.0, 34.0)))
                            .clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            self.root_input = path.display().to_string();
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.root_input)
                                .desired_width(370.0)
                                .hint_text("Folder path"),
                        );
                    });
                });
            });
    }

    fn sidebar(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .exact_size(236.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::same(18))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.label(section_title("VIEW"));
                ui.add_space(6.0);
                nav_button(ui, &mut self.view, View::Heatmap, "Heatmap");
                nav_button(ui, &mut self.view, View::Largest, "Largest files");
                nav_button(ui, &mut self.view, View::Duplicates, "Duplicates");

                ui.add_space(22.0);
                ui.label(section_title("FILTER"));
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .desired_width(f32::INFINITY)
                        .hint_text("Search files..."),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("File type").size(12.0).color(MUTED));
                let extensions = self.extensions();
                egui::ComboBox::from_id_salt("extension_filter")
                    .selected_text(&self.extension_filter)
                    .width(195.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.extension_filter,
                            "All types".to_owned(),
                            "All types",
                        );
                        for extension in extensions {
                            let label = extension.to_ascii_uppercase();
                            ui.selectable_value(&mut self.extension_filter, extension, label);
                        }
                    });
                ui.add_space(10.0);
                ui.label(RichText::new("Minimum size").size(12.0).color(MUTED));
                egui::ComboBox::from_id_salt("min_size")
                    .selected_text(["Any size", "1 MB+", "100 MB+", "1 GB+"][self.min_size_index])
                    .width(195.0)
                    .show_ui(ui, |ui| {
                        for (index, label) in ["Any size", "1 MB+", "100 MB+", "1 GB+"].iter().enumerate() {
                            ui.selectable_value(&mut self.min_size_index, index, *label);
                        }
                    });
                ui.add_space(10.0);
                ui.label(RichText::new("Modified").size(12.0).color(MUTED));
                egui::ComboBox::from_id_salt("age")
                    .selected_text(["Any time", "Last 30 days", "Last year", "Last 3 years"][self.age_index])
                    .width(195.0)
                    .show_ui(ui, |ui| {
                        for (index, label) in ["Any time", "Last 30 days", "Last year", "Last 3 years"].iter().enumerate() {
                            ui.selectable_value(&mut self.age_index, index, *label);
                        }
                    });

                ui.add_space(22.0);
                ui.label(section_title("SCAN OPTIONS"));
                ui.add_space(6.0);
                ui.checkbox(&mut self.find_duplicates, "Find exact duplicates");
                ui.checkbox(&mut self.include_hidden, "Include hidden files");
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Links are never followed. Files remain local and are only read during scanning.")
                        .size(11.0)
                        .color(MUTED),
                );

                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(RichText::new("LOCAL-FIRST  /  NO TELEMETRY").size(10.0).color(Color32::from_rgb(78, 116, 107)));
                });
            });
    }

    fn extensions(&self) -> Vec<String> {
        let Some(result) = &self.result else {
            return Vec::new();
        };
        let mut values: Vec<String> = result
            .files
            .iter()
            .map(|file| file.extension.clone())
            .collect();
        values.sort_unstable();
        values.dedup();
        values
    }

    fn details_panel(&mut self, root: &mut egui::Ui) {
        if self.result.is_none() {
            return;
        }
        let category_summary = self.selected_category.clone().map(|extension| {
            let indices = self.filtered_indices();
            let files: Vec<(PathBuf, u64)> = indices
                .into_iter()
                .filter_map(|index| self.result.as_ref()?.files.get(index))
                .filter(|file| file.extension == extension)
                .map(|file| (file.path.clone(), file.size))
                .collect();
            let total_bytes = files.iter().map(|(_, size)| *size).sum();
            let paths: Vec<PathBuf> = files.into_iter().map(|(path, _)| path).collect();
            (extension, paths, total_bytes)
        });
        egui::Panel::right("details")
            .exact_size(294.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::same(18))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(root, |ui| {
                ui.label(section_title("INSPECTOR"));
                ui.add_space(10.0);
                let file = self
                    .selected
                    .and_then(|index| self.result.as_ref()?.files.get(index).cloned());
                if let Some(file) = file {
                    let selected = self.selected.unwrap_or_default();
                    let color = color_for_extension(&file.extension);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
                        ui.painter().rect_filled(rect, 9.0, color);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            short_extension(&file.extension),
                            FontId::proportional(11.0),
                            Color32::WHITE,
                        );
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&file.name).strong().color(TEXT));
                            ui.label(
                                RichText::new(format_bytes(file.size))
                                    .size(13.0)
                                    .color(ACCENT),
                            );
                        });
                    });
                    ui.add_space(14.0);
                    detail_row(ui, "Type", &file.extension.to_ascii_uppercase());
                    detail_row(ui, "Modified", &relative_age(file.modified_unix));
                    detail_row(
                        ui,
                        "Status",
                        if self.kept.contains(&file.path) {
                            "Keep"
                        } else {
                            "Unreviewed"
                        },
                    );
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(file.path.display().to_string())
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.add_space(18.0);
                    if ui
                        .add_sized([258.0, 34.0], egui::Button::new("Show in file manager"))
                        .clicked()
                    {
                        self.reveal(selected);
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized([126.0, 32.0], egui::Button::new("Move..."))
                            .clicked()
                        {
                            self.move_selected(selected);
                        }
                        let kept = self.kept.contains(&file.path);
                        if ui
                            .add_sized(
                                [126.0, 32.0],
                                egui::Button::new(if kept { "Unmark" } else { "Keep" }),
                            )
                            .clicked()
                        {
                            if kept {
                                self.kept.remove(&file.path);
                            } else {
                                self.kept.insert(file.path.clone());
                            }
                        }
                    });
                    ui.add_space(5.0);
                    if ui
                        .add_sized(
                            [258.0, 34.0],
                            egui::Button::new(RichText::new("Move to Trash").color(DANGER)),
                        )
                        .clicked()
                    {
                        self.confirm_delete = Some(DeleteTarget::File(file.path.clone()));
                    }
                } else if let Some((extension, paths, total_bytes)) = &category_summary {
                    let color = color_for_extension(extension);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
                        ui.painter().rect_filled(rect, 9.0, color);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            short_extension(extension),
                            FontId::proportional(11.0),
                            Color32::WHITE,
                        );
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    ".{} category",
                                    extension.to_ascii_uppercase()
                                ))
                                .strong()
                                .color(TEXT),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} file{}  /  {}",
                                    paths.len(),
                                    if paths.len() == 1 { "" } else { "s" },
                                    format_bytes(*total_bytes)
                                ))
                                .size(13.0)
                                .color(ACCENT),
                            );
                        });
                    });
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(
                            "This category contains the files currently visible through your filters.",
                        )
                        .size(11.0)
                        .color(MUTED),
                    );
                    ui.add_space(12.0);
                    if ui
                        .add_sized(
                            [258.0, 36.0],
                            egui::Button::new(
                                RichText::new(format!(
                                    "Move {} visible file{} to Trash",
                                    paths.len(),
                                    if paths.len() == 1 { "" } else { "s" }
                                ))
                                .color(DANGER),
                            ),
                        )
                        .clicked()
                        && !paths.is_empty()
                    {
                        self.confirm_delete = Some(DeleteTarget::Category {
                            extension: extension.clone(),
                            paths: paths.clone(),
                            total_bytes: *total_bytes,
                        });
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            "Tip: use the left-side search, size, and date filters before selecting a category to narrow this action.",
                        )
                        .size(11.0)
                        .color(MUTED),
                    );
                } else {
                    ui.label(
                        RichText::new(
                            "Select a tile or row to inspect a file. Select a colored category header to manage that file type.",
                        )
                        .color(MUTED),
                    );
                    ui.add_space(18.0);
                    ui.label(
                        RichText::new("Tile area = file size")
                            .size(12.0)
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("Color = file type or age")
                            .size(12.0)
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("Double-click = reveal location")
                            .size(12.0)
                            .color(TEXT),
                    );
                }
            });
    }

    fn central(&mut self, root: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(root, |ui| {
                if self.scanning {
                    self.scanning_view(ui);
                } else if self.result.is_none() {
                    self.empty_view(ui);
                } else {
                    self.dashboard(ui);
                }
            });
    }

    fn empty_view(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.22).max(40.0));
            let (rect, _) = ui.allocate_exact_size(Vec2::new(120.0, 96.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(
                Rect::from_min_size(rect.min, Vec2::new(72.0, 54.0)),
                8.0,
                Color32::from_rgb(37, 104, 91),
            );
            painter.rect_filled(
                Rect::from_min_size(rect.min + Vec2::new(75.0, 0.0), Vec2::new(43.0, 54.0)),
                8.0,
                Color32::from_rgb(99, 81, 196),
            );
            painter.rect_filled(
                Rect::from_min_size(rect.min + Vec2::new(0.0, 57.0), Vec2::new(46.0, 37.0)),
                8.0,
                Color32::from_rgb(206, 132, 54),
            );
            painter.rect_filled(
                Rect::from_min_size(rect.min + Vec2::new(49.0, 57.0), Vec2::new(69.0, 37.0)),
                8.0,
                Color32::from_rgb(38, 122, 166),
            );
            ui.add_space(18.0);
            ui.label(
                RichText::new("Your storage, made visible")
                    .size(28.0)
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("Choose a folder to map every file by size, type, and age.")
                    .size(15.0)
                    .color(MUTED),
            );
            ui.add_space(22.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("Choose folder").strong().color(BG))
                        .fill(ACCENT)
                        .corner_radius(9)
                        .min_size(Vec2::new(150.0, 40.0)),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.root_input = path.display().to_string();
                self.begin_scan();
            }
        });
    }

    fn scanning_view(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.28).max(50.0));
            ui.spinner();
            ui.add_space(18.0);
            ui.label(
                RichText::new(self.progress.phase)
                    .size(24.0)
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "{} files  /  {}",
                    format_count(self.progress.files),
                    format_bytes(self.progress.bytes)
                ))
                .color(ACCENT),
            );
            ui.add_space(8.0);
            let path = self.progress.current.display().to_string();
            ui.label(RichText::new(ellipsize(&path, 100)).size(11.0).color(MUTED));
        });
    }

    fn dashboard(&mut self, ui: &mut egui::Ui) {
        self.metrics(ui);
        ui.add_space(12.0);
        match self.view {
            View::Heatmap => self.heatmap_view(ui),
            View::Largest => self.largest_view(ui),
            View::Duplicates => self.duplicates_view(ui),
        }
    }

    fn metrics(&self, ui: &mut egui::Ui) {
        let result = self.result.as_ref().unwrap();
        let filtered = self.filtered_indices();
        let filtered_bytes: u64 = filtered.iter().map(|index| result.files[*index].size).sum();
        let width = ((ui.available_width() - 24.0) / 3.0).max(120.0);
        ui.horizontal(|ui| {
            metric_card(
                ui,
                width,
                "VISIBLE SIZE",
                &format_bytes(filtered_bytes),
                &format!("{} total scanned", format_bytes(result.total_bytes)),
                ACCENT,
            );
            metric_card(
                ui,
                width,
                "FILES",
                &format_count(filtered.len()),
                &format!("{} skipped", result.skipped),
                Color32::from_rgb(120, 146, 255),
            );
            metric_card(
                ui,
                width,
                "DUPLICATE SPACE",
                &format_bytes(result.duplicate_bytes()),
                &format!("{} exact groups", result.duplicate_groups.len()),
                Color32::from_rgb(255, 174, 87),
            );
        });
    }

    fn heatmap_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("STORAGE HEATMAP")
                    .size(12.0)
                    .strong()
                    .color(MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.selectable_value(&mut self.color_mode, ColorMode::Age, "Color by age");
                ui.selectable_value(&mut self.color_mode, ColorMode::Type, "Color by type");
            });
        });
        ui.add_space(8.0);
        let desired = Vec2::new(ui.available_width(), ui.available_height().max(240.0));
        let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
        ui.painter().rect_filled(rect, 10.0, PANEL_2);
        let indices = self.filtered_indices();
        if indices.is_empty() {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "No files match these filters",
                FontId::proportional(15.0),
                MUTED,
            );
            return;
        }
        self.paint_heatmap(ui, rect.shrink(5.0), &indices);
    }

    fn paint_heatmap(&mut self, ui: &mut egui::Ui, rect: Rect, indices: &[usize]) {
        let Some(result) = &self.result else { return };
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for index in indices.iter().copied().take(1_500) {
            groups
                .entry(result.files[index].extension.clone())
                .or_default()
                .push(index);
        }
        let mut groups: Vec<(String, Vec<usize>, u64)> = groups
            .into_iter()
            .map(|(extension, files)| {
                let size = files
                    .iter()
                    .map(|index| result.files[*index].size.max(1))
                    .sum();
                (extension, files, size)
            })
            .collect();
        groups.sort_unstable_by_key(|group| std::cmp::Reverse(group.2));
        let weights: Vec<(usize, u64)> = groups
            .iter()
            .enumerate()
            .map(|(i, group)| (i, group.2))
            .collect();
        let group_rects = treemap::layout(&weights, rect);
        let mut clicked = None;
        let mut reveal = None;
        let mut category_clicked = None;

        for (group_index, group_rect) in group_rects {
            let (extension, file_indices, _) = &groups[group_index];
            let group_color = color_for_extension(extension);
            ui.painter()
                .rect_filled(group_rect, 5.0, group_color.gamma_multiply(0.16));
            ui.painter().rect_stroke(
                group_rect,
                5.0,
                Stroke::new(
                    if self.selected_category.as_ref() == Some(extension) {
                        2.0
                    } else {
                        1.0
                    },
                    if self.selected_category.as_ref() == Some(extension) {
                        Color32::WHITE
                    } else {
                        group_color.gamma_multiply(0.6)
                    },
                ),
                egui::StrokeKind::Inside,
            );
            let show_header = group_rect.width() > 72.0 && group_rect.height() > 48.0;
            let content_rect = if show_header {
                let header_rect = Rect::from_min_max(
                    group_rect.min,
                    egui::pos2(group_rect.max.x, group_rect.min.y + 23.0),
                );
                let category_response = ui.interact(
                    header_rect,
                    ui.make_persistent_id(("category", extension)),
                    Sense::click(),
                );
                category_response
                    .clone()
                    .on_hover_text("Select this file-type category");
                if category_response.clicked() {
                    category_clicked = Some(extension.clone());
                }
                ui.painter().text(
                    group_rect.min + Vec2::new(7.0, 6.0),
                    Align2::LEFT_TOP,
                    format!("{}  {}", short_extension(extension), file_indices.len()),
                    FontId::proportional(10.0),
                    group_color.gamma_multiply(1.45),
                );
                Rect::from_min_max(
                    group_rect.min + Vec2::new(4.0, 23.0),
                    group_rect.max - Vec2::splat(4.0),
                )
            } else {
                group_rect.shrink(2.0)
            };
            if content_rect.width() < 2.0 || content_rect.height() < 2.0 {
                continue;
            }
            let file_weights: Vec<(usize, u64)> = file_indices
                .iter()
                .map(|index| (*index, result.files[*index].size.max(1)))
                .collect();
            for (index, file_rect) in treemap::layout(&file_weights, content_rect) {
                let file = &result.files[index];
                let color = match self.color_mode {
                    ColorMode::Type => group_color,
                    ColorMode::Age => color_for_age(file.modified_unix),
                };
                let response = ui.interact(
                    file_rect,
                    ui.make_persistent_id(("file_tile", index)),
                    Sense::click(),
                );
                let hovered = response.hovered();
                let tile_color = if hovered {
                    color.gamma_multiply(1.18)
                } else {
                    color.gamma_multiply(0.88)
                };
                ui.painter().rect_filled(file_rect, 2.0, tile_color);
                if self.selected == Some(index) {
                    ui.painter().rect_stroke(
                        file_rect,
                        2.0,
                        Stroke::new(2.0, Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
                if file_rect.width() > 72.0 && file_rect.height() > 35.0 {
                    ui.painter().text(
                        file_rect.min + Vec2::new(5.0, 5.0),
                        Align2::LEFT_TOP,
                        ellipsize(&file.name, (file_rect.width() / 7.5) as usize),
                        FontId::proportional(10.0),
                        Color32::WHITE,
                    );
                    ui.painter().text(
                        file_rect.min + Vec2::new(5.0, 19.0),
                        Align2::LEFT_TOP,
                        format_bytes(file.size),
                        FontId::proportional(10.0),
                        Color32::from_white_alpha(205),
                    );
                }
                response.clone().on_hover_ui(|ui| {
                    ui.label(RichText::new(&file.name).strong());
                    ui.label(format!(
                        "{}  /  {}",
                        format_bytes(file.size),
                        file.extension.to_ascii_uppercase()
                    ));
                    ui.label(format!("Modified {}", relative_age(file.modified_unix)));
                    ui.label(
                        RichText::new(file.path.display().to_string())
                            .size(10.0)
                            .color(MUTED),
                    );
                });
                if response.clicked() {
                    clicked = Some(index);
                }
                if response.double_clicked() {
                    reveal = Some(index);
                }
            }
        }
        if let Some(index) = clicked {
            self.selected = Some(index);
            self.selected_category = None;
        } else if let Some(extension) = category_clicked {
            self.selected = None;
            self.selected_category = Some(extension);
        }
        if let Some(index) = reveal {
            self.reveal(index);
        }
    }

    fn largest_view(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("LARGEST FILES")
                .size(12.0)
                .strong()
                .color(MUTED),
        );
        ui.add_space(8.0);
        let indices = self.filtered_indices();
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (rank, index) in indices.iter().copied().take(2_000).enumerate() {
                    let file = &self.result.as_ref().unwrap().files[index];
                    let selected = self.selected == Some(index);
                    let response = egui::Frame::new()
                        .fill(if selected {
                            Color32::from_rgb(27, 49, 48)
                        } else {
                            PANEL_2
                        })
                        .corner_radius(7)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{:02}", rank + 1))
                                        .monospace()
                                        .color(MUTED),
                                );
                                ui.colored_label(
                                    color_for_extension(&file.extension),
                                    short_extension(&file.extension),
                                );
                                ui.label(RichText::new(&file.name).color(TEXT));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format_bytes(file.size))
                                            .strong()
                                            .color(ACCENT),
                                    );
                                    ui.label(
                                        RichText::new(relative_age(file.modified_unix))
                                            .size(11.0)
                                            .color(MUTED),
                                    );
                                });
                            });
                        })
                        .response
                        .interact(Sense::click());
                    if response.clicked() {
                        clicked = Some(index);
                    }
                    if response.double_clicked() {
                        clicked = Some(index);
                    }
                    ui.add_space(4.0);
                }
            });
        if let Some(index) = clicked {
            self.selected = Some(index);
            self.selected_category = None;
        }
    }

    fn duplicates_view(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("EXACT DUPLICATES")
                .size(12.0)
                .strong()
                .color(MUTED),
        );
        ui.add_space(5.0);
        ui.label(
            RichText::new("Matched by file size, sampled content, then a complete BLAKE3 hash.")
                .size(11.0)
                .color(MUTED),
        );
        ui.add_space(10.0);
        let Some(result) = &self.result else { return };
        if !self.find_duplicates || result.duplicate_groups.is_empty() {
            let message = if self.find_duplicates {
                "No exact duplicates found."
            } else {
                "Enable duplicate detection and scan again."
            };
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(message).color(MUTED));
            });
            return;
        }
        let groups = result.duplicate_groups.clone();
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (group_number, group) in groups.iter().enumerate() {
                    egui::Frame::new()
                        .fill(PANEL_2)
                        .corner_radius(9)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("GROUP {}", group_number + 1))
                                        .size(11.0)
                                        .strong()
                                        .color(Color32::from_rgb(255, 174, 87)),
                                );
                                ui.label(
                                    RichText::new(format!("{} copies", group.file_indices.len()))
                                        .size(11.0)
                                        .color(MUTED),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{} reclaimable",
                                            format_bytes(group.reclaimable())
                                        ))
                                        .strong()
                                        .color(ACCENT),
                                    );
                                });
                            });
                            ui.separator();
                            for (copy_number, index) in
                                group.file_indices.iter().copied().enumerate()
                            {
                                let Some(file) = self
                                    .result
                                    .as_ref()
                                    .and_then(|result| result.files.get(index))
                                else {
                                    continue;
                                };
                                let label = if copy_number == 0 {
                                    "KEEP ORIGINAL"
                                } else {
                                    "REVIEW COPY"
                                };
                                if ui
                                    .selectable_label(
                                        self.selected == Some(index),
                                        format!("{label}   {}", file.path.display()),
                                    )
                                    .clicked()
                                {
                                    clicked = Some(index);
                                }
                            }
                        });
                    ui.add_space(7.0);
                }
            });
        if let Some(index) = clicked {
            self.selected = Some(index);
            self.selected_category = None;
        }
    }

    fn confirm_dialog(&mut self, context: &egui::Context) {
        let Some(target) = self.confirm_delete.clone() else {
            return;
        };
        let (title, message, paths) = match &target {
            DeleteTarget::File(path) => (
                "Move file to Trash?",
                format!(
                    "“{}” will be moved to the operating system Trash / Recycle Bin.",
                    path.file_name()
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_default()
                ),
                vec![path.clone()],
            ),
            DeleteTarget::Category {
                extension,
                paths,
                total_bytes,
            } => (
                "Move category to Trash?",
                format!(
                    "Move {} visible .{} file{} ({}) to the operating system Trash / Recycle Bin? Review your active filters before continuing.",
                    paths.len(),
                    extension.to_ascii_uppercase(),
                    if paths.len() == 1 { "" } else { "s" },
                    format_bytes(*total_bytes)
                ),
                paths.clone(),
            ),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(context, |ui| {
                ui.set_width(390.0);
                ui.label(message);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.confirm_delete = None;
                    }
                    if ui
                        .add(egui::Button::new(
                            RichText::new("Move to Trash").color(DANGER),
                        ))
                        .clicked()
                    {
                        self.confirm_delete = None;
                        self.start_trash(paths.clone());
                    }
                });
            });
    }

    fn notice(&mut self, context: &egui::Context) {
        let Some((message, is_error, created)) = &self.notice else {
            return;
        };
        if created.elapsed() > Duration::from_secs(5) {
            self.notice = None;
            return;
        }
        let color = if *is_error { DANGER } else { ACCENT };
        egui::Area::new(egui::Id::new("notice"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-18.0, -18.0))
            .order(egui::Order::Foreground)
            .show(context, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(24, 30, 39))
                    .stroke(Stroke::new(1.0, color))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new(message).color(TEXT));
                    });
            });
    }
}

impl eframe::App for LumaDiskApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = root.ctx().clone();
        self.poll_scan();
        self.poll_trash();
        if self.scanning || self.trash_receiver.is_some() {
            context.request_repaint_after(Duration::from_millis(80));
        }
        self.header(root);
        self.sidebar(root);
        self.details_panel(root);
        self.central(root);
        self.confirm_dialog(&context);
        self.notice(&context);
    }
}

fn configure_style(context: &egui::Context) {
    context.set_theme(egui::Theme::Dark);
    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = Color32::from_rgb(11, 15, 21);
    style.visuals.faint_bg_color = PANEL_2;
    style.visuals.widgets.inactive.bg_fill = PANEL_2;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(31, 38, 49);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(73, 87, 108));
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(37, 48, 59);
    style.visuals.selection.bg_fill = Color32::from_rgb(34, 91, 77);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    context.set_style_of(egui::Theme::Dark, style);
}

fn nav_button(ui: &mut egui::Ui, current: &mut View, value: View, label: &str) {
    let selected = *current == value;
    if ui
        .add_sized(
            [198.0, 34.0],
            egui::Button::new(RichText::new(label).color(if selected { ACCENT } else { TEXT }))
                .selected(selected),
        )
        .clicked()
    {
        *current = value;
    }
}

fn section_title(text: &str) -> RichText {
    RichText::new(text).size(10.0).strong().color(MUTED)
}

fn metric_card(
    ui: &mut egui::Ui,
    width: f32,
    label: &str,
    value: &str,
    detail: &str,
    color: Color32,
) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(9)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(width - 26.0);
            ui.label(RichText::new(label).size(10.0).strong().color(MUTED));
            ui.label(RichText::new(value).size(21.0).strong().color(color));
            ui.label(RichText::new(detail).size(10.0).color(MUTED));
        });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).size(11.0).color(TEXT));
        });
    });
}

fn color_for_extension(extension: &str) -> Color32 {
    let palette = [
        Color32::from_rgb(32, 177, 132),
        Color32::from_rgb(48, 136, 202),
        Color32::from_rgb(109, 92, 222),
        Color32::from_rgb(218, 129, 51),
        Color32::from_rgb(204, 70, 112),
        Color32::from_rgb(38, 166, 184),
        Color32::from_rgb(133, 174, 65),
        Color32::from_rgb(181, 84, 198),
        Color32::from_rgb(207, 177, 57),
    ];
    let hash = extension.bytes().fold(5381_usize, |hash, byte| {
        hash.wrapping_mul(33) ^ byte as usize
    });
    palette[hash % palette.len()]
}

fn color_for_age(modified: i64) -> Color32 {
    let days = unix_now().saturating_sub(modified.max(0)) / 86_400;
    match days {
        0..=30 => Color32::from_rgb(49, 197, 146),
        31..=180 => Color32::from_rgb(70, 154, 195),
        181..=365 => Color32::from_rgb(116, 111, 200),
        366..=1095 => Color32::from_rgb(197, 133, 62),
        _ => Color32::from_rgb(190, 75, 91),
    }
}

fn short_extension(extension: &str) -> String {
    if extension == "no extension" {
        "FILE".to_owned()
    } else {
        extension
            .chars()
            .take(6)
            .collect::<String>()
            .to_ascii_uppercase()
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_count(value: usize) -> String {
    let text = value.to_string();
    let mut output = String::with_capacity(text.len() + text.len() / 3);
    for (index, character) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

fn relative_age(modified: i64) -> String {
    if modified <= 0 {
        return "unknown".to_owned();
    }
    let seconds = unix_now().saturating_sub(modified);
    match seconds {
        0..=3_599 => format!("{}m ago", (seconds / 60).max(1)),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        86_400..=2_592_000 => format!("{}d ago", seconds / 86_400),
        2_592_001..=31_536_000 => format!("{}mo ago", seconds / 2_592_000),
        _ => format!("{}y ago", seconds / 31_536_000),
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

fn ellipsize(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", text.chars().take(keep).collect::<String>())
}

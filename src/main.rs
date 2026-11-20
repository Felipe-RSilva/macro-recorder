#![windows_subsystem = "windows"]

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use enigo::{Button as EnigoButton, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
use serde::{Deserialize, Serialize};
mod config;
mod keys;

use config::Config;
use keys::key_from_name;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const BG: Color32 = Color32::from_rgb(15, 20, 29);
const PANEL: Color32 = Color32::from_rgb(23, 30, 42);
const PANEL_LIGHT: Color32 = Color32::from_rgb(31, 40, 55);
const TEXT: Color32 = Color32::from_rgb(231, 237, 247);
const MUTED: Color32 = Color32::from_rgb(142, 155, 176);
const ACCENT: Color32 = Color32::from_rgb(83, 164, 255);
const GREEN: Color32 = Color32::from_rgb(58, 202, 143);
const RED: Color32 = Color32::from_rgb(243, 93, 105);
const AMBER: Color32 = Color32::from_rgb(240, 178, 78);
const RECORDING_SAMPLE_INTERVAL_MS: u64 = 10;
const CUSTOM_SPEED_SELECTION: u32 = u32::MAX;
const FAST_PLAYBACK_THRESHOLD: u32 = 100;
const FAST_PLAYBACK_TICK_MS: u64 = 1;

#[cfg(target_os = "windows")]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetDoubleClickTime() -> u32;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MacroFile {
    format_version: u32,
    events: Vec<RecordedEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecordedEvent {
    at_ms: u64,
    x: i32,
    y: i32,
    action: Action,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Action {
    KeyDown { key: String },
    KeyUp { key: String },
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    Move,
}

#[derive(Debug)]
enum InputAction {
    KeyDown(String),
    KeyUp(String),
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    Move(i32, i32),
}

#[derive(Debug)]
enum InputMessage {
    Event(InputAction),
    Error(String),
}

enum UiMessage {
    PlaybackFinished,
    PlaybackError(String),
}

struct RecorderApp {
    input_rx: Receiver<InputMessage>,
    ui_tx: Sender<UiMessage>,
    ui_rx: Receiver<UiMessage>,
    events: Vec<RecordedEvent>,
    recording: bool,
    recording_started: Option<Instant>,
    /// Latest pointer position seen anywhere on screen (used for clicks).
    raw_position: (i32, i32),
    /// Latest pointer position outside this app's window (used as the final position).
    last_outside_position: Option<(i32, i32)>,
    window_rect: Option<egui::Rect>,
    playback_active: bool,
    playback_stop: Option<Arc<AtomicBool>>,
    status: String,
    error: Option<String>,
    record_hotkey: String,
    capturing_hotkey: bool,
    hotkey_held: bool,
    speed: u32,
    custom_speed: u32,
    repeats: u32,
    continuous: bool,
    current_file: Option<PathBuf>,
    screenshot_path: Option<PathBuf>,
    screenshot_requested: bool,
    screenshot_frames: u32,
}

impl RecorderApp {
    fn new(input_rx: Receiver<InputMessage>, screenshot_path: Option<PathBuf>) -> Self {
        let (ui_tx, ui_rx) = mpsc::channel();
        let config = Config::load();
        Self {
            input_rx,
            ui_tx,
            ui_rx,
            events: Vec::new(),
            recording: false,
            recording_started: None,
            raw_position: (0, 0),
            last_outside_position: None,
            window_rect: None,
            playback_active: false,
            playback_stop: None,
            status: "Pronto para gravar".to_owned(),
            error: None,
            record_hotkey: config.record_hotkey,
            capturing_hotkey: false,
            hotkey_held: false,
            speed: config.speed,
            custom_speed: config.custom_speed,
            repeats: config.repeats,
            continuous: config.continuous,
            current_file: None,
            screenshot_path,
            screenshot_requested: false,
            screenshot_frames: 0,
        }
    }

    fn start_recording(&mut self) {
        if self.playback_active {
            self.stop_playback();
        }
        self.events.clear();
        self.error = None;
        self.recording = true;
        self.recording_started = Some(Instant::now());
        self.last_outside_position = None;
        self.status = "Gravando cliques e teclas".to_owned();
    }

    fn stop_recording(&mut self) {
        if self.recording {
            // Remember where the pointer ended up, so playback finishes there.
            if let Some(position) = self.last_outside_position {
                if !self.events.is_empty() {
                    self.push_event(Action::Move, position);
                }
            }
            self.recording = false;
            self.recording_started = None;
            self.status = format!("Gravação parada · {} ações", self.events.len());
        }
    }

    fn stop_playback(&mut self) {
        if let Some(stop) = &self.playback_stop {
            stop.store(false, Ordering::SeqCst);
        }
        self.playback_active = false;
        self.status = "Reprodução parada".to_owned();
    }

    fn start_playback(&mut self) {
        if self.events.is_empty() {
            self.error = Some("Grave algo primeiro ou abra uma macro salva.".to_owned());
            return;
        }
        self.stop_recording();
        let stop = Arc::new(AtomicBool::new(true));
        self.playback_stop = Some(stop.clone());
        self.playback_active = true;
        self.error = None;
        self.status = if self.continuous {
            "Reproduzindo sem parar · Pause ou Scroll Lock para parar".to_owned()
        } else {
            format!("Reproduzindo {} ações", self.events.len())
        };

        let events = self.events.clone();
        let repeats = self.repeats.max(1);
        let speed = if self.speed == CUSTOM_SPEED_SELECTION {
            self.custom_speed.clamp(1, 100)
        } else {
            self.speed.min(FAST_PLAYBACK_THRESHOLD)
        };
        let continuous = self.continuous;
        let tx = self.ui_tx.clone();
        thread::spawn(move || {
            let result = replay_events(&events, repeats, speed, continuous, &stop);
            let message = match result {
                Ok(()) => UiMessage::PlaybackFinished,
                Err(problem) => UiMessage::PlaybackError(problem),
            };
            let _ = tx.send(message);
        });
    }

    fn save_macro(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Macro recording", &["json"])
            .set_file_name("my-macro.json")
            .save_file()
        else {
            return;
        };
        let file = MacroFile {
            format_version: 1,
            events: self.events.clone(),
        };
        match serde_json::to_vec_pretty(&file) {
            Ok(bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => {
                    self.current_file = Some(path.clone());
                    self.error = None;
                    self.status = format!("{} ações salvas", self.events.len());
                }
                Err(problem) => self.error = Some(format!("Could not save recording: {problem}")),
            },
            Err(problem) => self.error = Some(format!("Could not prepare recording: {problem}")),
        }
    }

    fn open_macro(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Macro recording", &["json"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<MacroFile>(&bytes) {
                Ok(file) if file.format_version == 1 => {
                    self.stop_recording();
                    self.stop_playback();
                    self.events = file.events;
                    self.current_file = Some(path);
                    self.error = None;
                    self.status = format!("{} ações abertas", self.events.len());
                }
                Ok(_) => self.error = Some("This macro uses a format version that is not supported.".to_owned()),
                Err(problem) => self.error = Some(format!("This file is not a readable macro: {problem}")),
            },
            Err(problem) => self.error = Some(format!("Could not open recording: {problem}")),
        }
    }

    fn handle_input(&mut self) {
        while let Ok(message) = self.input_rx.try_recv() {
            match message {
                InputMessage::Error(problem) => {
                    if self.recording {
                        self.stop_recording();
                    }
                    self.error = Some(format!("Keyboard and pointer listening stopped: {problem}"));
                }
                InputMessage::Event(action) => {
                    if let InputAction::KeyDown(key) | InputAction::KeyUp(key) = &action {
                        let pressed = matches!(action, InputAction::KeyDown(_));
                        if self.capturing_hotkey {
                            if pressed {
                                self.finish_hotkey_capture(key.clone());
                            }
                            continue;
                        }
                        if *key == self.record_hotkey {
                            if pressed && !self.hotkey_held {
                                self.hotkey_held = true;
                                if self.recording {
                                    self.stop_recording();
                                } else {
                                    self.start_recording();
                                }
                            } else if !pressed {
                                self.hotkey_held = false;
                            }
                            continue; // the shortcut itself is never part of a macro
                        }
                    }
                    let is_stop_key = matches!(
                        &action,
                        InputAction::KeyDown(key) if key == "Pause" || key == "ScrollLock"
                    );
                    if is_stop_key {
                        if self.recording {
                            self.stop_recording();
                            continue;
                        }
                        if self.playback_active {
                            self.stop_playback();
                            continue;
                        }
                    }
                    if let InputAction::Move(x, y) = action {
                        // Movement is only tracked, never recorded: just the last position matters.
                        self.raw_position = (x, y);
                        if !self.is_inside_window(x, y) {
                            self.last_outside_position = Some((x, y));
                        }
                    } else if self.recording {
                        let is_mouse_button = !matches!(action, InputAction::KeyDown(_) | InputAction::KeyUp(_));
                        if is_mouse_button && self.is_inside_window(self.raw_position.0, self.raw_position.1) {
                            continue; // clicks on this app's own buttons are not part of the macro
                        }
                        let action = match action {
                            InputAction::KeyDown(key) => Action::KeyDown { key },
                            InputAction::KeyUp(key) => Action::KeyUp { key },
                            InputAction::LeftDown => Action::LeftDown,
                            InputAction::LeftUp => Action::LeftUp,
                            InputAction::RightDown => Action::RightDown,
                            InputAction::RightUp => Action::RightUp,
                            InputAction::Move(_, _) => unreachable!(),
                        };
                        self.push_event(action, self.raw_position);
                    }
                }
            }
        }

        while let Ok(message) = self.ui_rx.try_recv() {
            self.playback_active = false;
            self.playback_stop = None;
            match message {
                UiMessage::PlaybackFinished => self.status = "Reprodução concluída".to_owned(),
                UiMessage::PlaybackError(problem) => {
                    self.status = "A reprodução não pôde continuar".to_owned();
                    self.error = Some(problem);
                }
            }
        }
    }

    fn finish_hotkey_capture(&mut self, key: String) {
        self.capturing_hotkey = false;
        if key == "Pause" || key == "ScrollLock" {
            self.error = Some("Pause e Scroll Lock são reservadas para parar. Escolha outra tecla.".to_owned());
        } else if key == "Escape" {
            self.status = "Troca de atalho cancelada".to_owned();
        } else {
            self.status = format!("Atalho de gravação: {}", key_label(&key));
            self.record_hotkey = key;
            self.error = None;
            self.save_config();
        }
    }

    fn save_config(&self) {
        Config {
            record_hotkey: self.record_hotkey.clone(),
            speed: self.speed,
            custom_speed: self.custom_speed,
            repeats: self.repeats,
            continuous: self.continuous,
        }
        .save();
    }

    fn is_inside_window(&self, x: i32, y: i32) -> bool {
        self.window_rect
            .is_some_and(|rect| rect.contains(egui::pos2(x as f32, y as f32)))
    }

    fn push_event(&mut self, action: Action, position: (i32, i32)) {
        let Some(started) = self.recording_started else {
            return;
        };
        self.events.push(RecordedEvent {
            at_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            x: position.0,
            y: position.1,
            action,
        });
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (record_text, record_fill) = if self.recording {
                ("■  Parar gravação", RED)
            } else {
                ("Gravar", Color32::from_rgb(120, 47, 58))
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(record_text).size(15.0).strong().color(TEXT))
                        .fill(record_fill)
                        .stroke(Stroke::new(1.0_f32, RED))
                        .rounding(10.0)
                        .min_size(Vec2::new(170.0, 46.0)),
                )
                .clicked()
            {
                if self.recording {
                    self.stop_recording();
                } else {
                    self.start_recording();
                }
            }

            let play_enabled = !self.events.is_empty() && !self.playback_active && !self.recording;
            if ui
                .add_enabled(
                    play_enabled,
                    egui::Button::new(RichText::new("▶  Reproduzir").size(15.0).strong().color(TEXT))
                        .fill(Color32::from_rgb(27, 88, 70))
                        .stroke(Stroke::new(1.0_f32, GREEN))
                        .rounding(10.0)
                        .min_size(Vec2::new(150.0, 46.0)),
                )
                .clicked()
            {
                self.start_playback();
            }

            if ui
                .add_enabled(
                    self.playback_active,
                    egui::Button::new(RichText::new("■  Parar").size(15.0).strong().color(TEXT))
                        .fill(PANEL_LIGHT)
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(72, 86, 108)))
                        .rounding(10.0)
                        .min_size(Vec2::new(100.0, 46.0)),
                )
                .clicked()
            {
                self.stop_playback();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.events.is_empty(),
                        egui::Button::new("Salvar").rounding(8.0).min_size(Vec2::new(72.0, 34.0)),
                    )
                    .clicked()
                {
                    self.save_macro();
                }
                if ui
                    .add(egui::Button::new("Abrir").rounding(8.0).min_size(Vec2::new(72.0, 34.0)))
                    .clicked()
                {
                    self.open_macro();
                }
            });
        });
    }

    fn show_stats(&self, ui: &mut egui::Ui) {
        let clicks = self
            .events
            .iter()
            .filter(|e| matches!(e.action, Action::LeftDown | Action::RightDown))
            .count();
        let keys = self
            .events
            .iter()
            .filter(|e| matches!(e.action, Action::KeyDown { .. }))
            .count();
        let duration = self.events.last().map_or(0, |e| e.at_ms);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (value, label, color) in [
                (clicks.to_string(), "CLIQUES", AMBER),
                (keys.to_string(), "TECLAS", GREEN),
                (format_duration(duration), "DURAÇÃO", ACCENT),
            ] {
                egui::Frame::none()
                    .fill(PANEL)
                    .rounding(10.0)
                    .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                    .show(ui, |ui| {
                        ui.set_min_width(96.0);
                        ui.label(RichText::new(value).size(18.0).strong().color(color));
                        ui.label(RichText::new(label).size(9.5).strong().color(MUTED));
                    });
            }
        });
    }

    fn show_event_list(&self, ui: &mut egui::Ui, height: f32) {
        egui::Frame::none()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(39, 49, 66)))
            .rounding(12.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.set_height(height);
                ui.set_width(ui.available_width());
                ui.label(RichText::new("AÇÕES GRAVADAS").size(10.5).strong().color(MUTED));
                ui.add_space(6.0);
                if self.events.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Nada gravado ainda").size(14.0).color(TEXT));
                        ui.label(
                            RichText::new("Clique em Gravar. Só cliques, teclas e a posição final do mouse são salvos.")
                                .size(11.5)
                                .color(MUTED),
                        );
                    });
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, 22.0, self.events.len(), |ui, range| {
                        for index in range {
                            let event = &self.events[index];
                            let (label, color) = describe_event(event);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{:>3}", index + 1)).monospace().size(11.0).color(MUTED),
                                );
                                ui.label(
                                    RichText::new(format_duration(event.at_ms)).monospace().size(11.0).color(MUTED),
                                );
                                ui.label(RichText::new(label).size(12.5).color(color));
                            });
                        }
                    });
            });
    }

    fn show_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot_path.as_ref() else {
            return;
        };
        self.screenshot_frames += 1;
        if self.screenshot_frames == 120 && !self.screenshot_requested {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        let captured = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = captured {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
            let [width, height] = image.size;
            let save_result = (|| -> Result<(), String> {
                if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent).map_err(|problem| problem.to_string())?;
                }
                image::save_buffer(path, &bytes, width as u32, height as u32, image::ColorType::Rgba8)
                    .map_err(|problem| problem.to_string())
            })();
            match save_result {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(problem) => {
                    self.error = Some(format!("Could not save screenshot: {problem}"));
                    self.screenshot_path = None;
                }
            }
        }
    }
}

impl eframe::App for RecorderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.window_rect = ctx.input(|input| input.viewport().outer_rect).map(|rect| {
            let scale = ctx.pixels_per_point();
            egui::Rect::from_min_max(rect.min * scale, rect.max * scale)
        });
        self.handle_input();
        ctx.request_repaint_after(Duration::from_millis(RECORDING_SAMPLE_INTERVAL_MS));

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(20.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Macro Recorder").size(22.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (label, color) = if self.recording {
                            ("GRAVANDO", RED)
                        } else if self.playback_active {
                            ("REPRODUZINDO", GREEN)
                        } else {
                            ("PRONTO", MUTED)
                        };
                        ui.label(RichText::new(label).size(11.0).strong().color(color));
                    });
                });
                ui.label(
                    RichText::new("Grava cliques, teclas e onde o mouse parou. Nada além disso.")
                        .size(12.5)
                        .color(MUTED),
                );
                ui.add_space(14.0);

                egui::Frame::none()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(39, 49, 66)))
                    .rounding(14.0)
                    .inner_margin(14.0)
                    .show(ui, |ui| self.show_toolbar(ui));
                ui.add_space(12.0);
                self.show_stats(ui);
                ui.add_space(12.0);

                let list_height = (ui.available_height() - 84.0).max(120.0);
                ui.horizontal_top(|ui| {
                    let side_width = 220.0;
                    let list_width = (ui.available_width() - side_width - 12.0).max(200.0);
                    ui.allocate_ui(Vec2::new(list_width, list_height + 26.0), |ui| {
                        self.show_event_list(ui, list_height);
                    });
                    ui.vertical(|ui| {
                        ui.set_width(side_width);
                        let before = (self.speed, self.custom_speed, self.repeats, self.continuous);
                        let card = |ui: &mut egui::Ui, title: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                            egui::Frame::none().fill(PANEL).rounding(12.0).inner_margin(14.0).show(ui, |ui| {
                                ui.set_width(side_width - 28.0);
                                ui.label(RichText::new(title).size(10.0).strong().color(MUTED));
                                ui.add_space(6.0);
                                add(ui);
                            });
                            ui.add_space(10.0);
                        };

                        card(ui, "ATALHO DE GRAVAÇÃO", &mut |ui| {
                            ui.horizontal(|ui| {
                                let shown = if self.capturing_hotkey {
                                    "Pressione uma tecla...".to_owned()
                                } else {
                                    key_label(&self.record_hotkey)
                                };
                                ui.label(RichText::new(shown).size(15.0).strong().color(ACCENT));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let text = if self.capturing_hotkey { "Cancelar" } else { "Alterar" };
                                    if ui.button(text).clicked() {
                                        self.capturing_hotkey = !self.capturing_hotkey;
                                        self.error = None;
                                    }
                                });
                            });
                            ui.label(RichText::new("Inicia e para a gravação. Esc cancela a troca.").size(10.5).color(MUTED));
                        });

                        card(ui, "VELOCIDADE DE REPRODUÇÃO", &mut |ui| {
                            egui::ComboBox::from_id_salt("playback_speed")
                                .selected_text(speed_label(self.speed, self.custom_speed))
                                .width(side_width - 36.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.speed, 1, "0.5x  ·  Lento");
                                    ui.selectable_value(&mut self.speed, 0, "1x  ·  Normal");
                                    ui.selectable_value(&mut self.speed, 2, "2x  ·  Rápido");
                                    ui.selectable_value(&mut self.speed, 100, "100x  ·  Máximo");
                                    ui.selectable_value(
                                        &mut self.speed,
                                        CUSTOM_SPEED_SELECTION,
                                        format!("Personalizado  ·  {}x", self.custom_speed),
                                    );
                                });
                            if self.speed == CUSTOM_SPEED_SELECTION {
                                ui.add(egui::Slider::new(&mut self.custom_speed, 1..=100).suffix("x"));
                            }
                        });

                        card(ui, "REPETIÇÃO", &mut |ui| {
                            ui.add_enabled_ui(!self.continuous, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Repetir").color(TEXT));
                                    ui.add(egui::DragValue::new(&mut self.repeats).range(1..=999_999).speed(0.2));
                                    ui.label(RichText::new("vez(es)").color(TEXT));
                                });
                            });
                            ui.add_space(4.0);
                            ui.checkbox(&mut self.continuous, "Repetir infinitamente");
                        });

                        if before != (self.speed, self.custom_speed, self.repeats, self.continuous) {
                            self.save_config();
                        }
                    });
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).size(12.0).color(if self.recording { RED } else { MUTED }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(path) = &self.current_file {
                            ui.label(RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).size(11.0).color(MUTED));
                        }
                    });
                });
                if let Some(problem) = &self.error {
                    ui.label(RichText::new(problem).size(12.0).color(RED));
                }
                ui.label(
                    RichText::new("Pause ou Scroll Lock param a gravação e a reprodução  ·  Licença MIT")
                        .size(10.0)
                        .color(MUTED),
                );
            });

        self.show_screenshot(ctx);
    }
}

fn key_label(key: &str) -> String {
    if let Some(letter) = key.strip_prefix("Key").filter(|rest| rest.len() == 1) {
        return letter.to_owned();
    }
    if let Some(digit) = key.strip_prefix("Num").filter(|rest| rest.len() == 1) {
        return digit.to_owned();
    }
    key.to_owned()
}

fn describe_event(event: &RecordedEvent) -> (String, Color32) {
    let at = format!("({}, {})", event.x, event.y);
    match &event.action {
        Action::LeftDown => (format!("Botão esquerdo pressionado  {at}"), AMBER),
        Action::LeftUp => (format!("Botão esquerdo solto  {at}"), AMBER),
        Action::RightDown => (format!("Botão direito pressionado  {at}"), AMBER),
        Action::RightUp => (format!("Botão direito solto  {at}"), AMBER),
        Action::KeyDown { key } => (format!("Tecla {key} pressionada"), GREEN),
        Action::KeyUp { key } => (format!("Tecla {key} solta"), GREEN),
        Action::Move => (format!("Posição final do mouse  {at}"), ACCENT),
    }
}

fn format_duration(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{:02}:{:02}.{:02}", seconds / 60, seconds % 60, (milliseconds % 1000) / 10)
}

fn speed_label(speed: u32, custom: u32) -> String {
    if speed == CUSTOM_SPEED_SELECTION {
        format!("Personalizado  ·  {custom}x")
    } else if speed == 0 {
        "1x".to_owned()
    } else if speed == 1 {
        "0.5x".to_owned()
    } else {
        format!("{speed}x")
    }
}

fn classify_input_event(event: rdev::EventType) -> Option<InputAction> {
    match event {
        rdev::EventType::KeyPress(key) => classify_key_event(key, true),
        rdev::EventType::KeyRelease(key) => classify_key_event(key, false),
        rdev::EventType::ButtonPress(rdev::Button::Left) => Some(InputAction::LeftDown),
        rdev::EventType::ButtonRelease(rdev::Button::Left) => Some(InputAction::LeftUp),
        rdev::EventType::ButtonPress(rdev::Button::Right) => Some(InputAction::RightDown),
        rdev::EventType::ButtonRelease(rdev::Button::Right) => Some(InputAction::RightUp),
        rdev::EventType::MouseMove { x, y } => {
            Some(InputAction::Move(x.round() as i32, y.round() as i32))
        }
        _ => None,
    }
}

fn classify_key_event(key: rdev::Key, pressed: bool) -> Option<InputAction> {
    let name = format!("{key:?}");
    let supported = key_from_name(&name).is_some() || name == "Pause" || name == "ScrollLock";
    if !supported {
        return None;
    }
    Some(if pressed {
        InputAction::KeyDown(name)
    } else {
        InputAction::KeyUp(name)
    })
}

fn listen_for_input(tx: Sender<InputMessage>) {
    thread::spawn(move || {
        let event_tx = tx.clone();
        let result = rdev::listen(move |event| {
            if let Some(action) = classify_input_event(event.event_type) {
                let _ = event_tx.send(InputMessage::Event(action));
            }
        });
        if let Err(problem) = result {
            let _ = tx.send(InputMessage::Error(format!("{problem:?}")));
        }
    });
}

fn playback_delay_ms(gap_ms: u64, speed: u32) -> u64 {
    match speed {
        0 => gap_ms,
        1 => gap_ms.saturating_mul(2),
        multiplier => gap_ms / u64::from(multiplier),
    }
}

#[cfg(target_os = "windows")]
fn double_click_pause() -> Duration {
    Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() / 2 }))
}

#[cfg(not(target_os = "windows"))]
fn double_click_pause() -> Duration {
    Duration::ZERO
}

fn replay_events(
    events: &[RecordedEvent],
    repeats: u32,
    speed: u32,
    continuous: bool,
    stop: &AtomicBool,
) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|problem| format!("Could not start playback: {problem}"))?;
    let mut pass = 0_u64;
    while stop.load(Ordering::SeqCst) && (continuous || pass < u64::from(repeats)) {
        let mut index = 0;
        let mut had_left_button_action = false;
        while index < events.len() {
            if !stop.load(Ordering::SeqCst) {
                break;
            }

            let event = &events[index];
            apply_action(&mut enigo, event)?;
            match &event.action {
                Action::LeftDown | Action::LeftUp => {
                    had_left_button_action = true;
                }
                _ => {}
            }
            index += 1;
            if index < events.len() {
                let delay_ms = if speed >= FAST_PLAYBACK_THRESHOLD {
                    FAST_PLAYBACK_TICK_MS
                } else {
                    let gap = events[index].at_ms.saturating_sub(event.at_ms);
                    playback_delay_ms(gap, speed)
                };
                if delay_ms > 0 {
                    thread::sleep(Duration::from_millis(delay_ms));
                }
            }
        }

        if !stop.load(Ordering::SeqCst) {
            break;
        }
        if had_left_button_action {
            thread::sleep(double_click_pause());
        }
        pass += 1;
    }
    Ok(())
}

fn apply_action(enigo: &mut Enigo, event: &RecordedEvent) -> Result<(), String> {
    let position = (event.x, event.y);
    if matches!(
        &event.action,
        Action::LeftDown | Action::LeftUp | Action::RightDown | Action::RightUp
    ) {
        enigo
            .move_mouse(position.0, position.1, Coordinate::Abs)
            .map_err(|problem| problem.to_string())?;
    }
    let result = match &event.action {
        Action::KeyDown { key } => key_from_name(key)
            .ok_or_else(|| format!("This recording contains an unsupported key: {key}"))
            .and_then(|key| enigo.key(key, Direction::Press).map_err(|problem| problem.to_string())),
        Action::KeyUp { key } => key_from_name(key)
            .ok_or_else(|| format!("This recording contains an unsupported key: {key}"))
            .and_then(|key| enigo.key(key, Direction::Release).map_err(|problem| problem.to_string())),
        Action::LeftDown => enigo.button(EnigoButton::Left, Direction::Press).map_err(|problem| problem.to_string()),
        Action::LeftUp => enigo.button(EnigoButton::Left, Direction::Release).map_err(|problem| problem.to_string()),
        Action::RightDown => enigo.button(EnigoButton::Right, Direction::Press).map_err(|problem| problem.to_string()),
        Action::RightUp => enigo.button(EnigoButton::Right, Direction::Release).map_err(|problem| problem.to_string()),
        Action::Move => enigo.move_mouse(position.0, position.1, Coordinate::Abs).map_err(|problem| problem.to_string()),
    };
    result
}

fn main() -> eframe::Result<()> {
    let mut screenshot_path = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--screenshot" {
            let Some(path) = args.next() else {
                eprintln!("--screenshot needs a PNG file name");
                std::process::exit(2);
            };
            screenshot_path = Some(PathBuf::from(path));
        }
    }

    let (input_tx, input_rx) = mpsc::channel();
    listen_for_input(input_tx);

    let screenshot_mode = screenshot_path.is_some();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Macro Recorder")
        .with_inner_size([760.0, 660.0])
        .with_min_inner_size([700.0, 620.0]);
    if screenshot_mode {
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::viewport::WindowLevel::AlwaysOnBottom);
        }
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    let options = {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        let mut options = options;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_activate_ignoring_other_apps(false);
        }));
        options
    };

    eframe::run_native(
        "Macro Recorder",
        options,
        Box::new(move |_creation_context| Ok(Box::new(RecorderApp::new(input_rx, screenshot_path)))),
    )
}

#[cfg(test)]
mod tests {
    use super::keys::key_from_name;
    use super::{
        classify_input_event, playback_delay_ms, InputAction,
    };

    #[test]
    fn original_speed_delay_rules_are_kept() {
        assert_eq!(playback_delay_ms(40, 0), 40);
        assert_eq!(playback_delay_ms(40, 1), 80);
        assert_eq!(playback_delay_ms(40, 2), 20);
        assert_eq!(playback_delay_ms(40, 8), 5);
    }

    #[test]
    fn only_supported_keys_are_accepted_for_recording() {
        assert!(key_from_name("KeyA").is_some());
        assert!(key_from_name("Escape").is_some());
        assert!(key_from_name("Unknown").is_none());
        assert!(key_from_name("Pause").is_none());
        assert!(key_from_name("ScrollLock").is_none());
    }

    #[test]
    fn input_filter_keeps_only_keys_pointer_and_primary_mouse_buttons() {
        assert!(matches!(
            classify_input_event(rdev::EventType::ButtonPress(rdev::Button::Left)),
            Some(InputAction::LeftDown)
        ));
        assert!(matches!(
            classify_input_event(rdev::EventType::ButtonRelease(rdev::Button::Right)),
            Some(InputAction::RightUp)
        ));
        assert!(classify_input_event(rdev::EventType::ButtonPress(rdev::Button::Middle)).is_none());
        // Plain mouse movement is classified (to track the last position) but never stored as an event.
        assert!(matches!(
            classify_input_event(rdev::EventType::MouseMove { x: 3.0, y: 4.0 }),
            Some(InputAction::Move(3, 4))
        ));
        assert!(classify_input_event(rdev::EventType::Wheel { delta_x: 0, delta_y: 1 }).is_none());
    }
}

//! In-app PhotoCraftAI chat panel.
//!
//! The panel talks to the local FastAPI bridge backend over SSE.  The backend
//! drives this same app through the control channel, so the user watches the
//! real menus, tools, dialogs and layers move while the log appears here.

use std::io::{BufRead, BufReader};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::Value;

#[derive(Default)]
struct Step {
    n: u64,
    tool: String,
    label: String,
    is_error: bool,
    result: String,
}

pub struct AiChat {
    open: bool,
    input: String,
    status: String,
    steps: Vec<Step>,
    busy: bool,
    rx: Option<Receiver<Value>>,
    cursor: Option<egui::Pos2>,
    active: bool,
    active_until: f64,
    reply: String,
    settings_open: bool,
    settings_loaded: bool,
    settings_rx: Option<Receiver<Value>>,
    settings_busy: bool,
    settings_status: String,
    provider: String,
    api_key: String,
    model: String,
    base_url: String,
    has_key: bool,
    masked_key: String,
}

impl Default for AiChat {
    fn default() -> Self {
        Self {
            open: true,
            input: String::new(),
            status: "输入一句话，例如：新建 1080x1920 画布，填充蓝色".into(),
            steps: Vec::new(),
            busy: false,
            rx: None,
            cursor: None,
            active: false,
            active_until: 0.0,
            reply: String::new(),
            settings_open: false,
            settings_loaded: false,
            settings_rx: None,
            settings_busy: false,
            settings_status: "填写 API Key 后，模型会用来理解你的需求。".into(),
            provider: "deepseek".into(),
            api_key: String::new(),
            model: "deepseek-chat".into(),
            base_url: "https://api.deepseek.com".into(),
            has_key: false,
            masked_key: String::new(),
        }
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn endpoint() -> String {
    std::env::var("PHOTOCRAFT_AI_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_string())
}

impl AiChat {
    fn send(&mut self) {
        let message = self.input.trim().to_string();
        if message.is_empty() || self.busy {
            return;
        }
        self.input.clear();
        self.steps.clear();
        self.reply.clear();
        self.busy = true;
        self.active = true;
        self.status = "正在连接 PhotoCraft…".into();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let url = format!("{}/chat/stream", endpoint().trim_end_matches('/'));

        std::thread::spawn(move || {
            let request = ureq::post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream");
            match request.send_json(serde_json::json!({ "message": message })) {
                Ok(response) => {
                    let reader = response.into_body().into_reader();
                    let mut reader = BufReader::new(reader);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) => break,
                            Ok(_) => {
                                if let Some(data) = line.strip_prefix("data: ")
                                    && let Ok(value) = serde_json::from_str::<Value>(data.trim())
                                {
                                    if tx.send(value).is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(error) => {
                                let _ = tx.send(serde_json::json!({
                                    "type": "error",
                                    "error": format!("读取后端流失败：{error}"),
                                }));
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.send(serde_json::json!({
                        "type": "error",
                        "error": format!("后端没启动或不可达：{error}"),
                    }));
                }
            }
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let mut events = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event.get("type").and_then(Value::as_str) {
                Some("status") => {
                    if let Some(text) = event.get("text").and_then(Value::as_str) {
                        self.status = text.to_string();
                    }
                }
                Some("step") => {
                    let n = event.get("step").and_then(Value::as_u64).unwrap_or(0);
                    let tool = event.get("tool").and_then(Value::as_str).unwrap_or("").to_string();
                    let label = event.get("label").and_then(Value::as_str).unwrap_or(&tool).to_string();
                    let is_error = event.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    let result = event
                        .get("result")
                        .map(|value| {
                            let text = value.to_string();
                            if text.chars().count() > 120 {
                                format!("{}…", text.chars().take(120).collect::<String>())
                            } else {
                                text
                            }
                        })
                        .unwrap_or_default();
                    self.steps.push(Step { n, tool, label, is_error, result });
                    self.active = true;
                    if let Some(cursor) = event.get("cursor")
                        && let (Some(x), Some(y)) = (
                            cursor.get("x").and_then(Value::as_f64),
                            cursor.get("y").and_then(Value::as_f64),
                        )
                    {
                        self.cursor = Some(egui::pos2(x as f32, y as f32));
                    }
                }
                Some("done") => {
                    self.busy = false;
                    self.status = "完成".into();
                    self.reply = event.get("reply").and_then(Value::as_str).unwrap_or("完成").to_string();
                    self.active_until = now() + 1.4;
                    self.rx = None;
                }
                Some("error") => {
                    self.busy = false;
                    self.status = "执行失败".into();
                    self.reply = event.get("error").and_then(Value::as_str).unwrap_or("执行失败").to_string();
                    self.active_until = now() + 1.2;
                    self.rx = None;
                }
                _ => {}
            }
        }

        if !self.busy && self.active_until > 0.0 && now() > self.active_until {
            self.active = false;
            self.active_until = 0.0;
            self.cursor = None;
        }
        if self.busy || self.active {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn fetch_settings(&mut self) {
        if self.settings_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.settings_rx = Some(rx);
        let url = format!("{}/settings", endpoint().trim_end_matches('/'));
        std::thread::spawn(move || {
            match ureq::get(&url).call() {
                Ok(mut response) => match response.body_mut().read_to_string() {
                    Ok(text) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            let _ = tx.send(value);
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(serde_json::json!({"type": "settings_error", "error": error.to_string()}));
                    }
                },
                Err(error) => {
                    let _ = tx.send(serde_json::json!({"type": "settings_error", "error": format!("后端不可达：{error}")}));
                }
            }
        });
    }

    fn save_settings(&mut self) {
        if self.settings_busy {
            return;
        }
        self.settings_busy = true;
        self.settings_status = "正在保存并测试连接…".into();
        let (tx, rx) = mpsc::channel();
        self.settings_rx = Some(rx);
        let url = format!("{}/settings", endpoint().trim_end_matches('/'));
        let test_url = format!("{}/settings/test", endpoint().trim_end_matches('/'));
        let body = serde_json::json!({
            "provider": self.provider,
            "api_key": self.api_key,
            "model": self.model,
            "base_url": self.base_url,
        });
        std::thread::spawn(move || {
            let saved = ureq::post(&url)
                .header("Content-Type", "application/json")
                .send_json(body);
            if let Err(error) = saved {
                let _ = tx.send(serde_json::json!({"type": "settings_error", "error": format!("保存失败：{error}")}));
                return;
            }
            match ureq::post(&test_url).send_empty() {
                Ok(mut response) => match response.body_mut().read_to_string() {
                    Ok(text) => {
                        if let Ok(mut value) = serde_json::from_str::<Value>(&text) {
                            if let Some(object) = value.as_object_mut() {
                                object.insert("type".into(), Value::String("settings_ok".into()));
                            }
                            let _ = tx.send(value);
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(serde_json::json!({"type": "settings_error", "error": error.to_string()}));
                    }
                },
                Err(ureq::Error::StatusCode(code)) => {
                    let _ = tx.send(serde_json::json!({
                        "type": "settings_error",
                        "error": format!("API Key 测试失败（HTTP {code}）：请检查 Key、模型和 Base URL"),
                    }));
                }
                Err(error) => {
                    let _ = tx.send(serde_json::json!({"type": "settings_error", "error": format!("测试失败：{error}")}));
                }
            }
        });
    }

    fn poll_settings(&mut self) {
        let mut events = Vec::new();
        if let Some(rx) = &self.settings_rx {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event.get("type").and_then(Value::as_str) {
                Some("settings_error") => {
                    self.settings_busy = false;
                    self.settings_status = event.get("error").and_then(Value::as_str).unwrap_or("设置失败").to_string();
                    self.settings_rx = None;
                }
                Some("settings_ok") => {
                    self.settings_busy = false;
                    self.settings_status = format!(
                        "连接成功：{} / {}",
                        event.get("provider").and_then(Value::as_str).unwrap_or(""),
                        event.get("model").and_then(Value::as_str).unwrap_or(""),
                    );
                    self.has_key = true;
                    self.settings_rx = None;
                }
                Some("error") => {
                    self.settings_busy = false;
                    self.settings_status = event.get("error").and_then(Value::as_str).unwrap_or("设置失败").to_string();
                    self.settings_rx = None;
                }
                _ => {
                    if let Some(provider) = event.get("provider").and_then(Value::as_str) {
                        self.provider = provider.to_string();
                    }
                    if let Some(model) = event.get("model").and_then(Value::as_str) {
                        self.model = model.to_string();
                    }
                    if let Some(base) = event.get("base_url").and_then(Value::as_str) {
                        self.base_url = base.to_string();
                    }
                    self.has_key = event.get("has_key").and_then(Value::as_bool).unwrap_or(false);
                    self.masked_key = event.get("masked_key").and_then(Value::as_str).unwrap_or("").to_string();
                    if self.has_key {
                        self.settings_status = format!("已保存 Key：{}", self.masked_key);
                    }
                    self.settings_rx = None;
                }
            }
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        egui::Window::new("PhotoCraftAI 设置")
            .id(egui::Id::unique("sayhua_ai_settings"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("API Key 只保存在本机 ai-agent/runtime/ai_settings.json。");
                ui.add_space(6.0);
                egui::Grid::new("pcai_settings_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("服务商");
                    egui::ComboBox::from_id_salt("pcai_provider")
                        .selected_text(match self.provider.as_str() {
                            "qwen" => "Qwen / 通义千问",
                            "openai" => "OpenAI",
                            _ => "DeepSeek",
                        })
                        .show_ui(ui, |ui| {
                            for (id, label, model, base) in [
                                ("deepseek", "DeepSeek", "deepseek-chat", "https://api.deepseek.com"),
                                ("qwen", "Qwen / 通义千问", "qwen-plus", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
                                ("openai", "OpenAI", "gpt-4o-mini", "https://api.openai.com/v1"),
                            ] {
                                if ui.selectable_label(self.provider == id, label).clicked() {
                                    self.provider = id.into();
                                    self.model = model.into();
                                    self.base_url = base.into();
                                }
                            }
                        });
                    ui.end_row();

                    ui.label("API Key");
                    ui.add(egui::TextEdit::singleline(&mut self.api_key).password(true).hint_text(
                        if self.has_key { format!("已保存：{}（留空保持不变）", self.masked_key) } else { "粘贴 API Key".into() }
                    ));
                    ui.end_row();

                    ui.label("模型");
                    ui.add(egui::TextEdit::singleline(&mut self.model));
                    ui.end_row();

                    ui.label("Base URL");
                    ui.add(egui::TextEdit::singleline(&mut self.base_url));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.settings_busy, egui::Button::new("保存并测试")).clicked() {
                        self.save_settings();
                    }
                    if self.settings_busy {
                        ui.spinner();
                    }
                });
                ui.label(egui::RichText::new(&self.settings_status).small().color(
                    if self.settings_status.contains("失败") || self.settings_status.contains("错误") {
                        egui::Color32::from_rgb(255, 130, 130)
                    } else {
                        egui::Color32::from_rgb(150, 190, 255)
                    },
                ));
            });
        self.settings_open = open;
    }

    pub fn panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.poll_settings();
        if !self.settings_loaded {
            self.settings_loaded = true;
            self.fetch_settings();
        }
        self.settings_window(&ctx);
        if !self.open {
            return;
        }
        egui::Panel::right("sayhua_ai_panel")
            .resizable(true)
            .default_size(372.0)
            .size_range(320.0..=540.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("PhotoCraftAI").strong().size(15.0));
                    ui.add_space(6.0);
                    if ui.small_button("设置").clicked() {
                        self.settings_open = true;
                    }
                    let color = if self.busy {
                        egui::Color32::from_rgb(110, 160, 255)
                    } else {
                        egui::Color32::from_rgb(90, 200, 150)
                    };
                    ui.colored_label(color, if self.busy { "AI 正在操作" } else { "就绪" });
                });
                ui.label(egui::RichText::new(if self.has_key { "Bridge · API Key 已配置" } else { "Bridge · 未配置 API Key" }).small().weak());
                ui.add_space(4.0);
                let label = if self.has_key {
                    "API Key 已配置 · 点此修改".to_string()
                } else {
                    "配置 API Key（接入 DeepSeek / Qwen / OpenAI）".to_string()
                };
                if ui.add_sized([ui.available_width(), 30.0], egui::Button::new(label)).clicked() {
                    self.settings_open = true;
                }
                if !self.has_key {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 185, 90),
                        "未配置 Key 时只能用本地规则理解简单指令",
                    );
                }
                ui.separator();

                let log_height = (ui.available_height() - 128.0).max(120.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), log_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sayhua_ai_log")
                    .auto_shrink([false, false])
                    .max_height(log_height)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&self.status).italics().weak());
                        for step in &self.steps {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("#{}", step.n))
                                        .monospace()
                                        .small()
                                        .weak(),
                                );
                                ui.label(egui::RichText::new(&step.label).strong());
                                ui.label(
                                    egui::RichText::new(&step.tool)
                                        .monospace()
                                        .small()
                                        .color(egui::Color32::from_rgb(130, 160, 230)),
                                );
                                let (text, color) = if step.is_error {
                                    ("is_error: true", egui::Color32::from_rgb(255, 120, 120))
                                } else {
                                    ("is_error: false", egui::Color32::from_rgb(90, 210, 150))
                                };
                                ui.label(egui::RichText::new(text).monospace().small().color(color));
                            });
                            if !step.result.is_empty() {
                                ui.label(egui::RichText::new(&step.result).small().weak());
                            }
                            ui.add_space(4.0);
                        }
                        if !self.reply.is_empty() {
                            ui.separator();
                            ui.label(egui::RichText::new(&self.reply).strong());
                        }
                    });
                    },
                );

                ui.separator();
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.input)
                        .hint_text("例如：新建 1080x1920 画布，填充蓝色")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                let enter = response.has_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
                ui.horizontal(|ui| {
                    let send = ui.add_enabled(!self.busy && !self.input.trim().is_empty(), egui::Button::new("发送"));
                    if send.clicked() || enter {
                        self.send();
                    }
                    ui.label(egui::RichText::new("Enter 发送 · Shift+Enter 换行").small().weak());
                });
                if self.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("正在驱动桌面版 PhotoCraft…").small().weak());
                    });
                }
            });
    }

    pub fn overlay(&mut self, ctx: &egui::Context) {
        self.draw_badge(ctx);
        self.draw_cursor(ctx);
    }

    fn draw_badge(&self, ctx: &egui::Context) {
        if !self.active {
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("sayhua_ai_badge"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(screen.left() + 18.0, screen.top() + 74.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_premultiplied(12, 22, 40, 235))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 130, 230)))
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("●").color(egui::Color32::from_rgb(100, 170, 255)));
                            ui.label(egui::RichText::new("AI 正在操作").color(egui::Color32::from_rgb(205, 225, 255)));
                        });
                    });
            });
    }

    fn draw_cursor(&self, ctx: &egui::Context) {
        if !self.active {
            return;
        }
        let Some(pos) = self.cursor else { return };
        egui::Area::new(egui::Id::unique("sayhua_ai_cursor"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let painter = ui.painter();
                let points = vec![
                    pos,
                    pos + egui::vec2(0.0, 20.0),
                    pos + egui::vec2(5.0, 15.0),
                    pos + egui::vec2(9.0, 23.0),
                    pos + egui::vec2(12.0, 21.5),
                    pos + egui::vec2(8.5, 13.5),
                    pos + egui::vec2(15.0, 13.5),
                ];
                painter.add(egui::Shape::convex_polygon(
                    points,
                    egui::Color32::WHITE,
                    egui::Stroke::new(1.2, egui::Color32::from_rgb(20, 28, 44)),
                ));
                painter.text(
                    pos + egui::vec2(15.0, 16.0),
                    egui::Align2::LEFT_TOP,
                    "AI",
                    egui::FontId::monospace(10.0),
                    egui::Color32::from_rgb(120, 165, 255),
                );
            });
    }
}

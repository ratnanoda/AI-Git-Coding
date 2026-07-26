#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui;
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum DownloadMode {
    Video,
    Audio,
}

struct App {
    url: String,
    output_dir: String,
    mode: DownloadMode,
    status: String,
    log: String,
    running: bool,
    receiver: Option<mpsc::Receiver<AppMessage>>,
    child: Arc<Mutex<Option<Child>>>,
}

enum AppMessage {
    Line(String),
    Finished(Result<(), String>),
}

impl Default for App {
    fn default() -> Self {
        let output_dir = std::env::var("USERPROFILE")
            .map(|home| format!("{}\\Downloads", home))
            .unwrap_or_else(|_| ".".to_owned());

        Self {
            url: String::new(),
            output_dir,
            mode: DownloadMode::Video,
            status: "準備完了".to_owned(),
            log: String::new(),
            running: false,
            receiver: None,
            child: Arc::new(Mutex::new(None)),
        }
    }
}

impl App {
    fn yt_dlp_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("yt-dlp.exe")
    }

    fn ensure_yt_dlp(&mut self) -> Result<PathBuf, String> {
        let path = Self::yt_dlp_path();
        if path.exists() {
            return Ok(path);
        }

        self.status = "yt-dlpを取得中...".to_owned();
        let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
        let escaped = path.to_string_lossy().replace('"', "\"");
        let script = format!(
            "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -Uri '{}' -OutFile \"{}\"",
            url, escaped
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
            .output()
            .map_err(|e| format!("PowerShellを起動できません: {e}"))?;

        if !output.status.success() || !path.exists() {
            return Err(format!(
                "yt-dlpの取得に失敗しました。\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(path)
    }

    fn start_download(&mut self) {
        if self.running {
            return;
        }
        if self.url.trim().is_empty() {
            self.status = "URLを入力してください".to_owned();
            return;
        }
        if self.output_dir.trim().is_empty() {
            self.status = "保存先を選択してください".to_owned();
            return;
        }

        let yt_dlp = match self.ensure_yt_dlp() {
            Ok(path) => path,
            Err(err) => {
                self.status = "エラー".to_owned();
                self.log = err;
                return;
            }
        };

        if let Err(err) = std::fs::create_dir_all(&self.output_dir) {
            self.status = "保存先を作成できません".to_owned();
            self.log = err.to_string();
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.running = true;
        self.status = "ダウンロード中...".to_owned();
        self.log.clear();

        let url = self.url.trim().to_owned();
        let output_template = PathBuf::from(&self.output_dir)
            .join("%(title)s.%(ext)s")
            .to_string_lossy()
            .to_string();
        let mode = self.mode;
        let shared_child = Arc::clone(&self.child);

        thread::spawn(move || {
            let mut command = Command::new(yt_dlp);
            command
                .arg("--newline")
                .arg("--no-playlist")
                .arg("--windows-filenames")
                .arg("-o")
                .arg(output_template);

            match mode {
                DownloadMode::Video => {
                    command.args(["-f", "best[ext=mp4]/best"]);
                }
                DownloadMode::Audio => {
                    command.args(["-f", "bestaudio[ext=m4a]/bestaudio"]);
                }
            }

            command
                .arg(url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .creation_flags(0x08000000);

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(e) => {
                    let _ = tx.send(AppMessage::Finished(Err(format!("yt-dlpを起動できません: {e}"))));
                    return;
                }
            };

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            if let Ok(mut slot) = shared_child.lock() {
                *slot = Some(child);
            }

            let tx_out = tx.clone();
            let out_thread = thread::spawn(move || {
                if let Some(stdout) = stdout {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        let _ = tx_out.send(AppMessage::Line(line));
                    }
                }
            });

            let tx_err = tx.clone();
            let err_thread = thread::spawn(move || {
                if let Some(stderr) = stderr {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let _ = tx_err.send(AppMessage::Line(line));
                    }
                }
            });

            let status = loop {
                let result = {
                    let mut guard = match shared_child.lock() {
                        Ok(g) => g,
                        Err(_) => {
                            let _ = tx.send(AppMessage::Finished(Err("内部エラーが発生しました".to_owned())));
                            return;
                        }
                    };
                    match guard.as_mut() {
                        Some(child) => child.try_wait(),
                        None => return,
                    }
                };

                match result {
                    Ok(Some(status)) => break status,
                    Ok(None) => thread::sleep(std::time::Duration::from_millis(100)),
                    Err(e) => {
                        let _ = tx.send(AppMessage::Finished(Err(format!("実行状態を確認できません: {e}"))));
                        return;
                    }
                }
            };

            let _ = out_thread.join();
            let _ = err_thread.join();
            if let Ok(mut slot) = shared_child.lock() {
                *slot = None;
            }

            if status.success() {
                let _ = tx.send(AppMessage::Finished(Ok(())));
            } else {
                let _ = tx.send(AppMessage::Finished(Err(format!("終了コード: {status}"))));
            }
        });
    }

    fn stop_download(&mut self) {
        if let Ok(mut slot) = self.child.lock() {
            if let Some(child) = slot.as_mut() {
                let _ = child.kill();
                self.status = "停止しました".to_owned();
            }
        }
    }

    fn poll_messages(&mut self) {
        let Some(rx) = &self.receiver else { return };
        while let Ok(message) = rx.try_recv() {
            match message {
                AppMessage::Line(line) => {
                    self.log.push_str(&line);
                    self.log.push('\n');
                }
                AppMessage::Finished(result) => {
                    self.running = false;
                    match result {
                        Ok(()) => {
                            self.status = "完了しました".to_owned();
                            self.log.push_str("\nダウンロードが完了しました。\n");
                        }
                        Err(err) => {
                            self.status = "失敗しました".to_owned();
                            self.log.push_str(&format!("\n{err}\n"));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
trait WindowsCommandExt {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}

#[cfg(target_os = "windows")]
impl WindowsCommandExt for Command {
    fn creation_flags(&mut self, flags: u32) -> &mut Self {
        use std::os::windows::process::CommandExt;
        CommandExt::creation_flags(self, flags)
    }
}

#[cfg(not(target_os = "windows"))]
trait WindowsCommandExt {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self;
}

#[cfg(not(target_os = "windows"))]
impl WindowsCommandExt for Command {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self {
        self
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages();
        if self.running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Rust yt-dlp GUI");
            ui.label("動画URLを貼り付けて、保存形式と保存先を選んでください。");
            ui.add_space(10.0);

            ui.label("URL");
            ui.add_enabled(
                !self.running,
                egui::TextEdit::singleline(&mut self.url)
                    .hint_text("https://www.youtube.com/watch?v=...")
                    .desired_width(f32::INFINITY),
            );

            ui.add_space(8.0);
            ui.label("保存形式");
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.running, |ui| {
                    ui.radio_value(&mut self.mode, DownloadMode::Video, "動画 (MP4)");
                    ui.radio_value(&mut self.mode, DownloadMode::Audio, "音声 (M4A)");
                });
            });

            ui.add_space(8.0);
            ui.label("保存先");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !self.running,
                    egui::TextEdit::singleline(&mut self.output_dir).desired_width(430.0),
                );
                if ui.add_enabled(!self.running, egui::Button::new("参照")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.output_dir = path.to_string_lossy().to_string();
                    }
                }
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.running, egui::Button::new("ダウンロード開始"))
                    .clicked()
                {
                    self.start_download();
                }
                if ui
                    .add_enabled(self.running, egui::Button::new("停止"))
                    .clicked()
                {
                    self.stop_download();
                }
                ui.label(format!("状態: {}", self.status));
            });

            ui.add_space(10.0);
            ui.label("ログ");
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(250.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.log)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(12)
                            .interactive(false),
                    );
                });

            ui.add_space(6.0);
            ui.small("初回実行時のみ、公式GitHub Releaseからyt-dlp.exeを自動取得します。");
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rust yt-dlp GUI")
            .with_inner_size([640.0, 520.0])
            .with_min_inner_size([560.0, 430.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Rust yt-dlp GUI",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::default()))
        }),
    )
}

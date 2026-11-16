//! egui window with Accounts, Presets, and Settings tabs.
//! Built only with `--features gui`. Launching, logging in, and the multi-instance
//! lock run in the background so the window keeps responding.

use crate::helper;
use crate::store::{self, Account, Preset, Settings};
use eframe::egui;
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, Receiver};
use std::thread;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Accounts,
    Presets,
    Settings,
}

/// A browser login in progress. The name may be blank, in which case the username is used.
struct PendingLogin {
    name: String,
    note: String,
    result: Receiver<Result<(String, String), String>>,
}

/// Things the window asks for during a frame. They're applied after the panels are drawn.
enum Action {
    SetTab(Tab),
    AddAccount,
    Login,
    RemoveAccount,
    Launch,
    NoteChanged,
    AddPreset,
    RemovePreset(usize),
    UsePreset(usize),
    ToggleMultiInstance,
    ApplyFps,
}

pub struct App {
    path: PathBuf,
    presets_path: PathBuf,
    settings_path: PathBuf,
    accounts: Vec<Account>,
    presets: Vec<Preset>,
    settings: Settings,
    tab: Tab,
    selected: Option<usize>,
    new_name: String,
    new_note: String,
    new_cookie: String,
    place_id: String,
    job_id: String,
    preset_name: String,
    preset_place: String,
    fps_text: String,
    helper_path: String,
    status: String,
    multi_child: Option<Child>,
    pending_launch: Option<Receiver<String>>,
    pending_login: Option<PendingLogin>,
}

impl App {
    pub fn new(path: PathBuf) -> Self {
        let mut status = String::new();
        let accounts = store::load(&path).unwrap_or_else(|e| {
            status = format!("Could not load accounts: {e}");
            Vec::new()
        });
        let presets_path = store::presets_path();
        let presets = store::load_presets(&presets_path).unwrap_or_default();
        let settings_path = store::settings_path();
        let settings = store::load_settings(&settings_path).unwrap_or_default();

        let mut app = Self {
            path,
            presets_path,
            settings_path,
            accounts,
            presets,
            settings: settings.clone(),
            tab: Tab::Accounts,
            selected: None,
            new_name: String::new(),
            new_note: String::new(),
            new_cookie: String::new(),
            place_id: String::new(),
            job_id: String::new(),
            preset_name: String::new(),
            preset_place: String::new(),
            fps_text: settings.fps_cap.to_string(),
            helper_path: helper::default_helper_path().to_string_lossy().into_owned(),
            status,
            multi_child: None,
            pending_launch: None,
            pending_login: None,
        };

        // Re-apply saved settings on startup.
        if settings.multi_instance {
            app.start_multi_instance();
        }
        if settings.fps_cap > 0 {
            let path = PathBuf::from(&app.helper_path);
            match helper::set_fps(&path, settings.fps_cap) {
                Ok(()) => {}
                Err(e) => app.status = format!("Could not apply saved FPS cap: {e}"),
            }
        }
        app
    }

    fn persist_accounts(&mut self) {
        if let Err(e) = store::save(&self.path, &self.accounts) {
            self.status = format!("Save failed: {e}");
        }
    }

    fn persist_presets(&mut self) {
        if let Err(e) = store::save_presets(&self.presets_path, &self.presets) {
            self.status = format!("Could not save presets: {e}");
        }
    }

    fn persist_settings(&mut self) {
        if let Err(e) = store::save_settings(&self.settings_path, &self.settings) {
            self.status = format!("Could not save settings: {e}");
        }
    }

    fn name_is_free(&self, name: &str) -> bool {
        !self
            .accounts
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Saves a new account. A blank name uses the username Roblox reports for the cookie.
    fn save_new_account(&mut self, typed_name: &str, note: String, blob: String, username: String) {
        let name = if typed_name.is_empty() {
            store::clean_field(&username)
        } else {
            typed_name.to_string()
        };
        if name.is_empty() {
            self.status =
                "Could not find a username for this cookie. Type a name and try again.".into();
            return;
        }
        if !self.name_is_free(&name) {
            self.status = format!("'{name}' already exists.");
            return;
        }
        self.accounts.push(Account {
            name: name.clone(),
            note,
            cookie_blob: blob,
        });
        self.selected = Some(self.accounts.len() - 1);
        self.persist_accounts();
        self.status = format!("Added {name}.");
    }

    fn add_account(&mut self) {
        let typed = store::clean_field(&self.new_name);
        let note = store::clean_field(&self.new_note);

        // Encrypt the pasted cookie before anything is stored. The field is cleared either way.
        let cookie = std::mem::take(&mut self.new_cookie);
        if cookie.trim().is_empty() {
            if typed.is_empty() {
                self.status = "Enter a name, or paste a cookie to use its username.".into();
                return;
            }
            if !self.name_is_free(&typed) {
                self.status = format!("'{typed}' already exists.");
                return;
            }
            self.save_new_account(&typed, note, String::new(), String::new());
            self.new_name.clear();
            self.new_note.clear();
            return;
        }

        match helper::protect(&PathBuf::from(&self.helper_path), &cookie) {
            Ok((blob, username)) => {
                self.save_new_account(&typed, note, blob, username);
                if self.status.starts_with("Added") {
                    self.new_name.clear();
                    self.new_note.clear();
                }
            }
            Err(e) => self.status = format!("Could not save cookie: {e}"),
        }
    }

    fn start_browser_login(&mut self) {
        if self.pending_login.is_some() {
            self.status = "A login window is already open.".into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        let helper_path = PathBuf::from(&self.helper_path);
        thread::spawn(move || {
            let _ = tx.send(helper::login(&helper_path));
        });

        self.pending_login = Some(PendingLogin {
            name: store::clean_field(&self.new_name),
            note: store::clean_field(&self.new_note),
            result: rx,
        });
        self.status = "Log in to Roblox in the window that opened...".into();
    }

    fn finish_browser_login(&mut self, result: Result<(String, String), String>) {
        let Some(login) = self.pending_login.take() else {
            return;
        };
        match result {
            Ok((blob, username)) => {
                self.save_new_account(&login.name, login.note, blob, username);
                if self.status.starts_with("Added") {
                    self.new_name.clear();
                    self.new_note.clear();
                }
            }
            Err(e) => self.status = format!("Login failed: {e}"),
        }
    }

    fn remove_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            if i < self.accounts.len() {
                let removed = self.accounts.remove(i);
                self.persist_accounts();
                self.status = format!("Removed {}.", removed.name);
            }
        }
    }

    fn add_preset(&mut self) {
        let name = store::clean_field(&self.preset_name);
        let place = store::clean_field(&self.preset_place);
        if name.is_empty() || place.parse::<u64>().is_err() {
            self.status = "A preset needs a name and a numeric place ID.".into();
            return;
        }
        self.presets.push(Preset {
            name: name.clone(),
            place_id: place,
        });
        self.preset_name.clear();
        self.preset_place.clear();
        self.persist_presets();
        self.status = format!("Saved preset {name}.");
    }

    fn start_multi_instance(&mut self) {
        self.stop_multi_instance();
        match helper::start_multi_instance(&PathBuf::from(&self.helper_path)) {
            Ok(child) => self.multi_child = Some(child),
            Err(e) => {
                self.settings.multi_instance = false;
                self.status = e;
            }
        }
    }

    fn stop_multi_instance(&mut self) {
        if let Some(mut child) = self.multi_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn toggle_multi_instance(&mut self) {
        if self.settings.multi_instance {
            self.start_multi_instance();
            if self.multi_child.is_some() {
                self.status =
                    "Multiple Roblox windows are allowed. Keep this app open while you play."
                        .into();
            }
        } else {
            self.stop_multi_instance();
            self.status = "Multiple Roblox windows are off for new launches.".into();
        }
        self.persist_settings();
    }

    fn apply_fps(&mut self) {
        match self.fps_text.trim().parse::<u32>() {
            Ok(fps) => {
                self.settings.fps_cap = fps;
                match helper::set_fps(&PathBuf::from(&self.helper_path), fps) {
                    Ok(()) => {
                        self.status = if fps == 0 {
                            "FPS cap removed. Restart Roblox to apply.".into()
                        } else {
                            format!("FPS cap set to {fps}. Restart Roblox to apply.")
                        };
                    }
                    Err(e) => self.status = format!("Could not set FPS cap: {e}"),
                }
                self.persist_settings();
            }
            Err(_) => self.status = "FPS cap must be a whole number (0 removes it).".into(),
        }
    }

    fn launch_selected(&mut self) {
        if self.pending_launch.is_some() {
            self.status = "Still launching the previous game. Please wait.".into();
            return;
        }
        let Some(account) = self.selected.and_then(|i| self.accounts.get(i)) else {
            self.status = "Select an account first.".into();
            return;
        };
        if account.cookie_blob.is_empty() {
            self.status = format!(
                "{} has no saved cookie. Log in again or add a cookie.",
                account.name
            );
            return;
        }
        let place_id = match self.place_id.trim().parse::<u64>() {
            Ok(id) => id,
            Err(_) => {
                self.status = "Place ID must be a number.".into();
                return;
            }
        };

        let (tx, rx) = mpsc::channel();
        let helper_path = PathBuf::from(&self.helper_path);
        let blob = account.cookie_blob.clone();
        let name = account.name.clone();
        let job_id = self.job_id.trim().to_string();
        let target = if job_id.is_empty() {
            "any server"
        } else {
            "that job"
        };
        self.status = format!("Launching {name} into {target}...");

        let thread_name = name.clone();
        thread::spawn(move || {
            let message = match helper::launch(&helper_path, &blob, place_id, &job_id) {
                Ok(()) => format!("Launched {thread_name} in place {place_id}."),
                Err(e) => format!("Launch failed for {thread_name}: {e}"),
            };
            let _ = tx.send(message);
        });

        self.pending_launch = Some(rx);
    }

    fn accounts_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.heading("Selected account");
        match self.selected.and_then(|i| self.accounts.get_mut(i)) {
            Some(account) => {
                ui.label(format!("Name: {}", account.name));
                let cookie_status = if account.cookie_blob.is_empty() {
                    "none (launching needs a cookie)"
                } else {
                    "saved (encrypted)"
                };
                ui.label(format!("Cookie: {cookie_status}"));
                ui.horizontal(|ui| {
                    ui.label("Note:");
                    if ui.text_edit_singleline(&mut account.note).changed() {
                        actions.push(Action::NoteChanged);
                    }
                });
                if ui.button("Remove account").clicked() {
                    actions.push(Action::RemoveAccount);
                }
            }
            None => {
                ui.label("Select an account on the left, or add one below.");
            }
        }

        ui.separator();
        ui.heading("Launch");
        ui.add(egui::TextEdit::singleline(&mut self.place_id).hint_text("Place ID"));
        ui.add(
            egui::TextEdit::singleline(&mut self.job_id)
                .hint_text("Job ID (optional, joins that exact server)"),
        );
        if ui.button("Launch selected account").clicked() {
            actions.push(Action::Launch);
        }

        ui.separator();
        ui.heading("Add account");
        ui.label("Leave the name blank to use the username Roblox reports.");
        ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("Account name (optional)"));
        ui.add(egui::TextEdit::singleline(&mut self.new_note).hint_text("Note (optional)"));
        ui.horizontal(|ui| {
            if ui.button("Log in with browser").clicked() {
                actions.push(Action::Login);
            }
            if ui.button("Add with pasted cookie").clicked() {
                actions.push(Action::AddAccount);
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.new_cookie)
                .hint_text("Or paste a .ROBLOSECURITY cookie")
                .password(true),
        );
    }

    fn presets_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.heading("Presets");
        if self.presets.is_empty() {
            ui.label("No presets yet. Save a place ID below.");
        }
        for (i, preset) in self.presets.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}  ({})", preset.name, preset.place_id));
                if ui.button("Use").clicked() {
                    actions.push(Action::UsePreset(i));
                }
                if ui.button("Remove").clicked() {
                    actions.push(Action::RemovePreset(i));
                }
            });
        }

        ui.separator();
        ui.heading("Add preset");
        ui.add(egui::TextEdit::singleline(&mut self.preset_name).hint_text("Preset name"));
        ui.add(egui::TextEdit::singleline(&mut self.preset_place).hint_text("Place ID"));
        if ui.button("Save preset").clicked() {
            actions.push(Action::AddPreset);
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.heading("Settings");
        if ui
            .checkbox(
                &mut self.settings.multi_instance,
                "Allow multiple Roblox windows",
            )
            .changed()
        {
            actions.push(Action::ToggleMultiInstance);
        }
        ui.label("Keep this app open while multiple windows are running.");

        ui.separator();
        ui.label("FPS cap for every Roblox window (0 removes it). Roblox must restart to apply.");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.fps_text).desired_width(80.0));
            if ui.button("Apply FPS cap").clicked() {
                actions.push(Action::ApplyFps);
            }
        });

        ui.separator();
        ui.collapsing("Helper settings", |ui| {
            ui.label("Path to RobloxProcessLookup.exe:");
            ui.text_edit_singleline(&mut self.helper_path);
        });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop_multi_instance();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pick up background results.
        let launch_done = self
            .pending_launch
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some(message) = launch_done {
            self.status = message;
            self.pending_launch = None;
        }
        let login_done = self
            .pending_login
            .as_ref()
            .and_then(|p| p.result.try_recv().ok());
        if let Some(result) = login_done {
            self.finish_browser_login(result);
        }
        if self.pending_launch.is_some() || self.pending_login.is_some() {
            ctx.request_repaint();
        }

        let mut actions: Vec<Action> = Vec::new();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Accounts, "Accounts"),
                    (Tab::Presets, "Presets"),
                    (Tab::Settings, "Settings"),
                ] {
                    if ui.selectable_label(self.tab == tab, label).clicked() {
                        actions.push(Action::SetTab(tab));
                    }
                }
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label(self.status.as_str());
        });

        egui::SidePanel::left("accounts")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("Accounts");
                ui.label(format!("{} saved", self.accounts.len()));
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, account) in self.accounts.iter().enumerate() {
                        let label = if account.note.is_empty() {
                            account.name.clone()
                        } else {
                            format!("{}  -  {}", account.name, account.note)
                        };
                        if ui
                            .selectable_label(self.selected == Some(i), label)
                            .clicked()
                        {
                            self.selected = Some(i);
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Accounts => self.accounts_tab(ui, &mut actions),
            Tab::Presets => self.presets_tab(ui, &mut actions),
            Tab::Settings => self.settings_tab(ui, &mut actions),
        });

        for action in actions {
            match action {
                Action::SetTab(tab) => self.tab = tab,
                Action::AddAccount => self.add_account(),
                Action::Login => self.start_browser_login(),
                Action::RemoveAccount => self.remove_selected(),
                Action::Launch => self.launch_selected(),
                Action::NoteChanged => self.persist_accounts(),
                Action::AddPreset => self.add_preset(),
                Action::RemovePreset(i) => {
                    if i < self.presets.len() {
                        let removed = self.presets.remove(i);
                        self.persist_presets();
                        self.status = format!("Removed preset {}.", removed.name);
                    }
                }
                Action::UsePreset(i) => {
                    if let Some(preset) = self.presets.get(i) {
                        self.place_id = preset.place_id.clone();
                        self.tab = Tab::Accounts;
                        self.status =
                            format!("Place set to {} ({}).", preset.name, preset.place_id);
                    }
                }
                Action::ToggleMultiInstance => self.toggle_multi_instance(),
                Action::ApplyFps => self.apply_fps(),
            }
        }
    }
}

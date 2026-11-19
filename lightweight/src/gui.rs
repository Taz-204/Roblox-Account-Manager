//! egui window with Accounts, Presets, and Settings tabs.
//! Built only with `--features gui`. Launching, logging in, avatars, and the
//! multi-instance lock run in the background so the window keeps responding.

use crate::helper::{self, AccountInfo};
use crate::monitor::{self, Limits, Monitor};
use crate::store::{self, Account, Preset, Settings};
use crate::winapi;
use eframe::egui;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::process::Child;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Home,
    Accounts,
    Presets,
    Settings,
}

/// A browser login in progress. The name may be blank, in which case the username is used.
struct PendingLogin {
    name: String,
    note: String,
    result: Receiver<Result<AccountInfo, String>>,
}

/// Things the window asks for during a frame. They're applied after the panels are drawn.
enum Action {
    SetTab(Tab),
    Select {
        index: usize,
        shift: bool,
        multi: bool,
    },
    AddAccount,
    Login,
    RemoveSelected,
    Launch,
    PerformanceLaunch,
    NoteChanged,
    AddPreset,
    RemovePreset(usize),
    UsePreset(usize),
    ToggleMultiInstance,
    ApplyFps,
    ApplyLaunchDelay,
    ApplyGraphics,
    ApplyLimits,
    HomeClick {
        pid: u32,
        ctrl: bool,
    },
    HomeAdd(u32),
    ClearHomeSelection,
    KillClients(Vec<u32>),
    ApplyTheme,
    ApplyHomeCap,
    SetFlag(String, bool),
    RemoveHomeCap,
    LowerPriority,
    ApplyFlags,
    Spread,
    Resize,
    ToggleCpuShare,
}

pub struct App {
    path: PathBuf,
    presets_path: PathBuf,
    settings_path: PathBuf,
    accounts: Vec<Account>,
    presets: Vec<Preset>,
    settings: Settings,
    tab: Tab,
    /// Indexes of the selected accounts, and the last one clicked (for Shift-click ranges).
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    new_name: String,
    new_note: String,
    new_cookie: String,
    place_id: String,
    job_id: String,
    preset_name: String,
    preset_place: String,
    fps_text: String,
    delay_text: String,
    graphics_text: String,
    flags_text: String,
    resize_width: String,
    resize_height: String,
    helper_path: String,
    status: String,
    avatars: HashMap<String, Vec<u8>>,
    avatar_started: HashSet<String>,
    multi_child: Option<Child>,
    pending_launch: Option<Receiver<String>>,
    pending_login: Option<PendingLogin>,
    monitor: Monitor,
    /// Process ID of each client this app started, mapped to the account name.
    launched: Arc<Mutex<HashMap<u32, String>>>,
    ram_text: String,
    cpu_text: String,
    home_selected: BTreeSet<u32>,
    home_cap_text: String,
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
        let helper_path = helper::default_helper_path().to_string_lossy().into_owned();
        let monitor = Monitor::start(Limits {
            ram_mb: settings.ram_limit_mb,
            cpu_percent: settings.cpu_limit_percent,
        });
        let ram_text = settings.ram_limit_mb.to_string();
        let cpu_text = settings.cpu_limit_percent.to_string();

        let mut app = Self {
            path,
            presets_path,
            settings_path,
            accounts,
            presets,
            settings: settings.clone(),
            tab: Tab::Home,
            selected: BTreeSet::new(),
            anchor: None,
            new_name: String::new(),
            new_note: String::new(),
            new_cookie: String::new(),
            place_id: String::new(),
            job_id: String::new(),
            preset_name: String::new(),
            preset_place: String::new(),
            fps_text: settings.fps_cap.to_string(),
            delay_text: settings.launch_delay_ms.to_string(),
            graphics_text: settings.graphics_level.to_string(),
            resize_width: "256".into(),
            resize_height: "144".into(),
            flags_text: store::read_text(&store::flags_path()).unwrap_or_default(),
            helper_path,
            status,
            avatars: HashMap::new(),
            avatar_started: HashSet::new(),
            multi_child: None,
            pending_launch: None,
            pending_login: None,
            monitor,
            launched: Arc::new(Mutex::new(HashMap::new())),
            ram_text,
            cpu_text,
            home_selected: BTreeSet::new(),
            home_cap_text: "512".to_string(),
        };

        // Re-apply saved settings on startup.
        if settings.multi_instance {
            app.start_multi_instance();
        }
        if settings.fps_cap > 0 {
            if let Err(e) = helper::set_fps(&PathBuf::from(&app.helper_path), settings.fps_cap) {
                app.status = format!("Could not apply saved FPS cap: {e}");
            }
        }
        if !app.flags_text.trim().is_empty() {
            if let Err(e) = helper::apply_flags(&PathBuf::from(&app.helper_path), &app.flags_text) {
                app.status = format!("Could not apply saved flags: {e}");
            }
        }
        if settings.graphics_level > 0 {
            if let Err(e) =
                helper::set_graphics(&PathBuf::from(&app.helper_path), settings.graphics_level)
            {
                app.status = format!("Could not apply saved graphics level: {e}");
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

    fn select_only(&mut self, index: usize) {
        self.selected = BTreeSet::from([index]);
        self.anchor = Some(index);
    }

    fn select(&mut self, index: usize, shift: bool, multi: bool) {
        if shift {
            let anchor = self.anchor.unwrap_or(index);
            self.selected = (anchor.min(index)..=anchor.max(index)).collect();
            self.anchor = Some(anchor);
        } else if multi {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(index);
        } else {
            self.select_only(index);
        }
    }

    /// The account the details panel and note editor show.
    fn primary(&self) -> Option<usize> {
        self.anchor
            .filter(|a| self.selected.contains(a))
            .or_else(|| self.selected.iter().next().copied())
    }

    /// Saves a new account. A blank name uses the username Roblox reports for the cookie.
    fn save_new_account(
        &mut self,
        typed_name: &str,
        note: String,
        cookie_blob: String,
        username: String,
        user_id: String,
    ) -> bool {
        let name = if typed_name.is_empty() {
            store::clean_field(&username)
        } else {
            typed_name.to_string()
        };
        if name.is_empty() {
            self.status =
                "Could not find a username for this cookie. Type a name and try again.".into();
            return false;
        }
        if !self.name_is_free(&name) {
            self.status = format!("'{name}' already exists.");
            return false;
        }
        self.accounts.push(Account {
            name: name.clone(),
            note,
            cookie_blob,
            user_id,
        });
        let index = self.accounts.len() - 1;
        self.select_only(index);
        self.persist_accounts();
        self.status = format!("Added {name}.");
        true
    }

    fn add_account(&mut self) {
        let typed = store::clean_field(&self.new_name);
        let note = store::clean_field(&self.new_note);

        // Encrypt the pasted cookie before anything is stored. The field is cleared either way.
        let cookie = std::mem::take(&mut self.new_cookie);
        let saved = if cookie.trim().is_empty() {
            self.save_new_account(&typed, note, String::new(), String::new(), String::new())
        } else {
            match helper::protect(&PathBuf::from(&self.helper_path), &cookie) {
                Ok(info) => {
                    self.save_new_account(&typed, note, info.blob, info.username, info.user_id)
                }
                Err(e) => {
                    self.status = format!("Could not save cookie: {e}");
                    false
                }
            }
        };
        if saved {
            self.new_name.clear();
            self.new_note.clear();
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

    fn finish_browser_login(&mut self, result: Result<AccountInfo, String>) {
        let Some(login) = self.pending_login.take() else {
            return;
        };
        match result {
            Ok(info) => {
                let saved = self.save_new_account(
                    &login.name,
                    login.note,
                    info.blob,
                    info.username,
                    info.user_id,
                );
                if saved {
                    self.new_name.clear();
                    self.new_note.clear();
                }
            }
            Err(e) => self.status = format!("Login failed: {e}"),
        }
    }

    fn remove_selected(&mut self) {
        // Remove from the end so earlier indexes stay valid.
        let indexes: Vec<usize> = self.selected.iter().rev().copied().collect();
        let mut removed = Vec::new();
        for i in indexes {
            if i < self.accounts.len() {
                removed.push(self.accounts.remove(i).name);
            }
        }
        self.selected.clear();
        self.anchor = None;
        if !removed.is_empty() {
            self.persist_accounts();
            self.status = format!("Removed {}.", removed.join(", "));
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

    fn apply_launch_delay(&mut self) {
        match self.delay_text.trim().parse::<u64>() {
            Ok(ms) => {
                self.settings.launch_delay_ms = ms;
                self.persist_settings();
                self.status = format!("Launch delay set to {ms} ms.");
            }
            Err(_) => self.status = "Launch delay must be a whole number of milliseconds.".into(),
        }
    }

    fn apply_graphics(&mut self) {
        match self.graphics_text.trim().parse::<u32>() {
            Ok(level) if level <= 10 => {
                self.settings.graphics_level = level;
                match helper::set_graphics(&PathBuf::from(&self.helper_path), level) {
                    Ok(()) => {
                        self.status = if level == 0 {
                            "Graphics override removed. Restart Roblox to apply.".into()
                        } else {
                            format!("Graphics quality set to {level}. Restart Roblox to apply.")
                        };
                    }
                    Err(e) => self.status = format!("Could not set graphics quality: {e}"),
                }
                self.persist_settings();
            }
            _ => self.status = "Graphics quality must be 0 to 10 (0 removes the override).".into(),
        }
    }

    /// Account names whose launched client is currently running.
    fn active_names(&self) -> HashSet<String> {
        let running: HashSet<u32> = self.procs_snapshot().iter().map(|p| p.pid).collect();
        self.launched
            .lock()
            .map(|map| {
                map.iter()
                    .filter(|(pid, _)| running.contains(pid))
                    .map(|(_, name)| name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn procs_snapshot(&self) -> Vec<monitor::ProcInfo> {
        self.monitor
            .procs
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    fn home_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.heading("Home");
        let procs = self.procs_snapshot();
        let names: HashMap<u32, String> =
            self.launched.lock().map(|m| m.clone()).unwrap_or_default();

        let total_cpu: f32 = procs.iter().map(|p| p.cpu_percent).sum();
        let total_mem: f32 = procs.iter().map(|p| p.memory_mb).sum();
        ui.label(format!(
            "{} running client(s). Total CPU {:.0}%, memory {:.0} MB. Updates every second.",
            procs.len(),
            total_cpu,
            total_mem
        ));
        ui.horizontal(|ui| {
            let count = self.home_selected.len();
            if ui
                .add_enabled(
                    count > 0,
                    egui::Button::new(format!("Kill selected ({count})")),
                )
                .clicked()
            {
                actions.push(Action::KillClients(
                    self.home_selected.iter().copied().collect(),
                ));
            }
            if ui
                .add_enabled(count > 0, egui::Button::new("Clear selection"))
                .clicked()
            {
                actions.push(Action::ClearHomeSelection);
            }
            ui.label("Drag across rows to select several. Ctrl+click adds or removes one.");
        });
        ui.horizontal(|ui| {
            ui.label("RAM cap for selected clients (MB):");
            ui.add(egui::TextEdit::singleline(&mut self.home_cap_text).desired_width(70.0));
            let count = self.home_selected.len();
            if ui
                .add_enabled(count > 0, egui::Button::new("Apply cap"))
                .clicked()
            {
                actions.push(Action::ApplyHomeCap);
            }
            if ui
                .add_enabled(count > 0, egui::Button::new("Remove cap"))
                .clicked()
            {
                actions.push(Action::RemoveHomeCap);
            }
        });
        ui.separator();

        if procs.is_empty() {
            ui.label("No Roblox clients are running.");
            return;
        }

        let pointer_down = ui.input(|i| i.pointer.primary_down());
        let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in &procs {
                let name = names
                    .get(&p.pid)
                    .cloned()
                    .unwrap_or_else(|| "Started outside this app".into());
                let selected = self.home_selected.contains(&p.pid);
                ui.horizontal(|ui| {
                    let response = ui.add_sized(
                        [240.0, 22.0],
                        egui::SelectableLabel::new(selected, format!("{name}  (PID {})", p.pid)),
                    );
                    // Dragging across rows selects each one. Ctrl-click is handled on click.
                    if response.clicked() {
                        actions.push(Action::HomeClick { pid: p.pid, ctrl });
                    } else if pointer_down && !ctrl && response.hovered() && !selected {
                        actions.push(Action::HomeAdd(p.pid));
                    }
                    ui.label(format!("{:.1}% CPU", p.cpu_percent));
                    ui.label(format!("{:.0} MB", p.memory_mb));
                    let mut limits = Vec::new();
                    if p.cpu_limited {
                        limits.push("CPU");
                    }
                    if p.ram_limited {
                        limits.push("RAM");
                    }
                    ui.label(if limits.is_empty() {
                        "no limits".to_string()
                    } else {
                        format!("limited: {}", limits.join(", "))
                    });
                    if ui.button("Kill client").clicked() {
                        actions.push(Action::KillClients(vec![p.pid]));
                    }
                });
            }
        });
    }

    /// Caps the RAM of every selected client at the value typed on the Home tab.
    fn apply_home_cap(&mut self) {
        let mb = match self.home_cap_text.trim().parse::<u32>() {
            Ok(mb) if mb >= 64 => mb,
            _ => {
                self.status = "RAM cap must be a whole number of at least 64 MB.".into();
                return;
            }
        };
        self.set_caps(Some(mb));
    }

    /// Sets (Some) or removes (None) the RAM cap on every selected client.
    fn set_caps(&mut self, mb: Option<u32>) {
        let pids: Vec<u32> = self.home_selected.iter().copied().collect();
        if pids.is_empty() {
            self.status = "Select clients on the Home tab first.".into();
            return;
        }
        if let Ok(mut caps) = self.monitor.ram_overrides.lock() {
            for pid in &pids {
                match mb {
                    Some(mb) => {
                        caps.insert(*pid, mb);
                    }
                    None => {
                        caps.remove(pid);
                    }
                }
            }
        }
        self.status = match mb {
            Some(mb) => format!(
                "Capped {} client(s) at {mb} MB. The cap takes effect within a second.",
                pids.len()
            ),
            None => format!(
                "Removed the cap from {} client(s). The global RAM limit applies again.",
                pids.len()
            ),
        };
    }

    fn kill_clients(&mut self, pids: &[u32]) {
        let mut killed = 0;
        let mut failed = Vec::new();
        for &pid in pids {
            match winapi::kill_process(pid) {
                Ok(()) => killed += 1,
                Err(e) => failed.push(format!("PID {pid}: {e}")),
            }
            self.home_selected.remove(&pid);
        }
        self.status = if failed.is_empty() {
            format!("Stopped {killed} client(s). Anything unsaved in them is lost.")
        } else {
            format!("Stopped {killed}. Could not stop: {}", failed.join("; "))
        };
    }

    fn apply_limits(&mut self) {
        let ram = match self.ram_text.trim().parse::<u32>() {
            Ok(0) => 0,
            Ok(mb) if mb >= 64 => mb,
            _ => {
                self.status = "RAM limit must be 0 (off) or at least 64 MB.".into();
                return;
            }
        };
        let cpu = match self.cpu_text.trim().parse::<u32>() {
            Ok(p) if p <= 100 => p,
            _ => {
                self.status = "CPU limit must be 0 (off) to 100 percent.".into();
                return;
            }
        };
        self.settings.ram_limit_mb = ram;
        self.settings.cpu_limit_percent = cpu;
        self.persist_settings();
        if let Ok(mut limits) = self.monitor.limits.lock() {
            *limits = Limits {
                ram_mb: ram,
                cpu_percent: cpu,
            };
        }
        self.status = "Limits saved. They now apply to running and new Roblox clients.".into();
    }

    /// Every fast flag the app can set: (category, flag name, value). Each one is a checkbox in Settings.
    const FAST_FLAGS: &[(&str, &str, &str)] = &[
        ("Shadows", "FIntRenderShadowIntensity", "0"),
        (
            "Shadows",
            "DFIntCullFactorPixelThresholdShadowMapHighQuality",
            "2147483647",
        ),
        (
            "Shadows",
            "DFIntCullFactorPixelThresholdShadowMapLowQuality",
            "2147483647",
        ),
        ("Lighting", "FFlagDisablePostFx", "True"),
        ("Lighting", "FFlagRenderNoLowFrmBloom", "False"),
        ("Lighting", "FIntRenderLocalLightFadeInMs", "0"),
        ("Lighting", "FIntRenderLocalLightUpdatesMax", "8"),
        ("Lighting", "FIntRenderLocalLightUpdatesMin", "6"),
        (
            "Textures (may not take effect)",
            "DFFlagTextureQualityOverrideEnabled",
            "True",
        ),
        (
            "Textures (may not take effect)",
            "DFIntTextureQualityOverride",
            "0",
        ),
        (
            "Animations (characters may freeze)",
            "DFIntMaxActiveAnimationTracks",
            "0",
        ),
        (
            "Animations (characters may freeze)",
            "FIntGrassMovementReducedMotionFactor",
            "0",
        ),
        (
            "Extra (caps FPS at 10)",
            "DFFlagDebugPauseVoxelizer",
            "True",
        ),
        (
            "Extra (caps FPS at 10)",
            "DFIntTaskSchedulerTargetFps",
            "10",
        ),
        ("Extra (caps FPS at 10)", "FFlagDebugSkyGray", "True"),
        ("Extra (caps FPS at 10)", "FIntFRMMaxGrassDistance", "0"),
        ("Extra (caps FPS at 10)", "FIntFRMMinGrassDistance", "0"),
    ];

    /// Turns one fast flag on or off in the saved list.
    fn set_flag(&mut self, name: &str, on: bool) {
        let mut names: Vec<String> = self
            .settings
            .enabled_flags
            .split(',')
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect();
        names.retain(|f| f != name);
        if on {
            names.push(name.to_string());
        }
        self.settings.enabled_flags = names.join(",");
    }

    /// The flags sent to Roblox: the typed flags plus any enabled presets. Presets are added last.
    fn effective_flags(&self) -> String {
        let raw = self.flags_text.trim();
        let mut parts: Vec<String> = Vec::new();
        if let Some(inner) = raw.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            let inner = inner.trim();
            if !inner.is_empty() {
                parts.push(inner.to_string());
            }
        } else if !raw.is_empty() {
            // Not a JSON object. Send it unchanged so the helper reports the error.
            return raw.to_string();
        }
        for (_, name, value) in Self::FAST_FLAGS {
            if self
                .settings
                .enabled_flags
                .split(',')
                .any(|f| f.trim() == *name)
            {
                parts.push(format!("\"{name}\": \"{value}\""));
            }
        }
        if parts.is_empty() {
            "{}".to_string()
        } else {
            format!("{{{}}}", parts.join(", "))
        }
    }

    fn apply_flags(&mut self) {
        // The helper checks that the JSON is a valid flags object. Only a accepted set is saved,
        // so a bad entry is never retried at startup.
        let text = self.flags_text.trim().to_string();
        let helper_path = PathBuf::from(&self.helper_path);
        let effective = self.effective_flags();
        match helper::apply_flags(&helper_path, &effective) {
            Ok(()) => match store::write_text(&store::flags_path(), &text) {
                Ok(()) => self.status = "Flags saved and written. Restart Roblox to apply.".into(),
                Err(e) => self.status = format!("Flags applied, but could not save them: {e}"),
            },
            Err(e) => self.status = format!("Flags rejected: {e}"),
        }
    }

    /// Applies the saved FPS, graphics, and flag settings in the background, then launches the selected accounts.
    /// Nothing is launched if a setting fails to apply.
    fn performance_launch(&mut self) {
        let flags = self.effective_flags();
        self.launch_selected_with(Some((
            Some((self.settings.fps_cap, self.settings.graphics_level)),
            flags,
        )));
    }

    fn resize_windows(&mut self) {
        let (width, height) = match (
            self.resize_width.trim().parse::<u32>(),
            self.resize_height.trim().parse::<u32>(),
        ) {
            (Ok(w), Ok(h)) => (w, h),
            _ => {
                self.status = "Width and height must be whole numbers.".into();
                return;
            }
        };
        match winapi::resize_windows(width, height) {
            Ok(applied) if applied.is_empty() => {
                self.status = "No running Roblox windows to resize.".into()
            }
            Ok(sizes) => {
                self.status = format!(
                    "Asked for {width}x{height}. Windows now: {}. Roblox may enforce a minimum size.",
                    sizes.join(", ")
                );
            }
            Err(e) => self.status = format!("Could not resize windows: {e}"),
        }
    }

    fn spread(&mut self) {
        match winapi::spread_across_cores() {
            Ok(count) => {
                self.status = format!(
                    "Spread {} running Roblox window(s) across CPU cores.",
                    count
                )
            }
            Err(e) => self.status = format!("Could not spread windows: {e}"),
        }
    }

    fn lower_priority(&mut self) {
        let normal = self.settings.cpu_priority_normal;
        match winapi::set_priority(normal) {
            Ok(count) => {
                self.status = format!("Lowered priority of {} running Roblox window(s).", count)
            }
            Err(e) => self.status = format!("Could not lower priority: {e}"),
        }
    }

    /// Starts the selected accounts one after another, waiting the launch delay between each.
    fn launch_selected(&mut self) {
        // Typed flags and enabled fast flags are applied before every launch.
        let flags = self.effective_flags();
        let prelaunch = if flags == "{}" {
            None
        } else {
            Some((None, flags))
        };
        self.launch_selected_with(prelaunch);
    }

    /// `prelaunch` is (optional FPS cap and graphics level, flags). They are applied in the background before any launch.
    fn launch_selected_with(&mut self, prelaunch: Option<(Option<(u32, u32)>, String)>) {
        if self.pending_launch.is_some() {
            self.status = "Still launching the previous games. Please wait.".into();
            return;
        }
        let targets: Vec<(String, String)> = self
            .selected
            .iter()
            .filter_map(|&i| self.accounts.get(i))
            .filter(|a| !a.cookie_blob.is_empty())
            .map(|a| (a.name.clone(), a.cookie_blob.clone()))
            .collect();
        if targets.is_empty() {
            self.status = "Select an account that has a saved cookie first.".into();
            return;
        }
        let skipped = self.selected.len() - targets.len();

        let place_id = match self.place_id.trim().parse::<u64>() {
            Ok(id) => id,
            Err(_) => {
                self.status = "Place ID must be a number.".into();
                return;
            }
        };
        let job_id = self.job_id.trim().to_string();
        let delay = Duration::from_millis(self.settings.launch_delay_ms);
        let helper_path = PathBuf::from(&self.helper_path);
        let count = targets.len();
        let cpu_share = self.settings.cpu_share_on_launch;
        let cpu_normal = self.settings.cpu_priority_normal;

        // Roblox runs only one client unless the multi-instance lock is held, so a second client
        // closes right away. Turn the lock on when several accounts start together.
        if count > 1 && self.multi_child.is_none() {
            self.start_multi_instance();
            if self.multi_child.is_some() {
                self.settings.multi_instance = true;
                self.persist_settings();
            }
        }

        let launched_map = self.launched.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            if let Some((fps_graphics, flags)) = prelaunch {
                let mut problems = Vec::new();
                if let Some((fps, graphics)) = fps_graphics {
                    if let Err(e) = helper::set_fps(&helper_path, fps) {
                        problems.push(format!("FPS: {e}"));
                    }
                    if let Err(e) = helper::set_graphics(&helper_path, graphics) {
                        problems.push(format!("graphics: {e}"));
                    }
                }
                if !flags.is_empty() {
                    if let Err(e) = helper::apply_flags(&helper_path, &flags) {
                        problems.push(format!("flags: {e}"));
                    }
                }
                if !problems.is_empty() {
                    let _ = tx.send(format!(
                        "Not launched. Could not apply settings: {}",
                        problems.join("; ")
                    ));
                    return;
                }
            }
            let mut launched = 0;
            let mut started: Vec<(String, u32)> = Vec::new();
            let mut failed = Vec::new();
            for (i, (name, blob)) in targets.into_iter().enumerate() {
                if i > 0 && !delay.is_zero() {
                    thread::sleep(delay);
                }
                match helper::launch_fast(&helper_path, &blob, place_id, &job_id) {
                    Ok(pid) => {
                        launched += 1;
                        if pid > 0 {
                            if let Ok(mut map) = launched_map.lock() {
                                map.insert(pid, name.clone());
                            }
                            started.push((name.clone(), pid));
                        }
                    }
                    Err(e) => failed.push(format!("{name}: {e}")),
                }
            }
            let mut message = format!("Launched {launched} of {count} in place {place_id}.");
            if cpu_share && launched > 0 {
                message.push_str(" CPU sharing applies in about 20 seconds.");
            }
            if skipped > 0 {
                message.push_str(&format!(" {skipped} without a cookie were skipped."));
            }
            if !failed.is_empty() {
                message.push_str(&format!(" Failed: {}", failed.join("; ")));
            }
            if !started.is_empty() {
                // Give the clients a few seconds to start, then check they are still running.
                thread::sleep(Duration::from_secs(5));
                let running = winapi::roblox_process_ids();
                let closed: Vec<String> = started
                    .iter()
                    .filter(|(_, pid)| !running.contains(pid))
                    .map(|(name, _)| name.clone())
                    .collect();
                if !closed.is_empty() {
                    message.push_str(&format!(
                        " These closed right after starting: {}. Turn on multi-instance if more than one client is open.",
                        closed.join(", ")
                    ));
                }
            }
            let _ = tx.send(message);

            if cpu_share && launched > 0 {
                // Roblox needs a moment to open its windows before they can be adjusted.
                thread::sleep(Duration::from_secs(20));
                let _ = winapi::set_priority(cpu_normal);
                let _ = winapi::spread_across_cores();
            }
        });

        self.pending_launch = Some(rx);
        self.status = format!("Launching {count} account(s) in place {place_id}...");
    }

    /// Loads saved headshots and downloads any that are missing. Returns true while some are still missing.
    fn refresh_avatars(&mut self) -> bool {
        let mut to_fetch: Vec<(String, PathBuf)> = Vec::new();
        let mut missing = false;

        for account in &self.accounts {
            let uid = &account.user_id;
            if uid.is_empty() || self.avatars.contains_key(uid) {
                continue;
            }
            let Some(path) = store::avatar_path(uid) else {
                continue;
            };
            if let Ok(bytes) = std::fs::read(&path) {
                self.avatars.insert(uid.clone(), bytes);
                continue;
            }
            missing = true;
            if !self.avatar_started.contains(uid) {
                to_fetch.push((uid.clone(), path));
            }
        }

        for (uid, path) in to_fetch {
            self.avatar_started.insert(uid.clone());
            let helper_path = PathBuf::from(&self.helper_path);
            thread::spawn(move || {
                let _ = helper::avatar(&helper_path, &uid, &path);
            });
        }
        missing
    }

    fn accounts_list(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let active = self.active_names();
        for (i, account) in self.accounts.iter().enumerate() {
            let is_selected = self.selected.contains(&i);
            let stroke = if is_selected {
                egui::Stroke::new(
                    2.0,
                    if ui.visuals().dark_mode {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::BLACK
                    },
                )
            } else {
                egui::Stroke::NONE
            };

            egui::Frame::none()
                .stroke(stroke)
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::same(4.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let size = egui::vec2(32.0, 32.0);
                        match self.avatars.get(&account.user_id) {
                            Some(bytes) => {
                                let uri = format!("bytes://avatar/{}", account.user_id);
                                ui.add(
                                    egui::Image::from_bytes(uri, bytes.clone())
                                        .fit_to_exact_size(size),
                                );
                            }
                            None => {
                                let _ = ui.allocate_space(size);
                            }
                        }

                        let label = if account.note.is_empty() {
                            account.name.clone()
                        } else {
                            format!("{}  -  {}", account.name, account.note)
                        };
                        let response = ui.selectable_label(is_selected, label);
                        if active.contains(&account.name) {
                            ui.colored_label(egui::Color32::from_rgb(40, 160, 60), "Active");
                        } else {
                            ui.colored_label(egui::Color32::GRAY, "Inactive");
                        }
                        if response.clicked() {
                            let modifiers = ui.input(|input| input.modifiers);
                            actions.push(Action::Select {
                                index: i,
                                shift: modifiers.shift,
                                multi: modifiers.ctrl || modifiers.command,
                            });
                        }
                    });
                });
            ui.add_space(2.0);
        }
    }

    fn accounts_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.heading("Selected account");
        if self.selected.len() > 1 {
            ui.label(format!(
                "{} accounts selected. Launch starts all of them.",
                self.selected.len()
            ));
        } else if let Some(account) = self.primary().and_then(|i| self.accounts.get_mut(i)) {
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
        } else {
            ui.label("Click an account on the left. Ctrl-click or Shift-click to select several.");
        }
        if !self.selected.is_empty() && ui.button("Remove selected").clicked() {
            actions.push(Action::RemoveSelected);
        }

        ui.separator();
        ui.heading("Launch");
        ui.add(egui::TextEdit::singleline(&mut self.place_id).hint_text("Place ID"));
        ui.add(
            egui::TextEdit::singleline(&mut self.job_id)
                .hint_text("Job ID (optional, joins that exact server)"),
        );
        let label = if self.selected.len() > 1 {
            format!("Launch {} selected accounts", self.selected.len())
        } else {
            "Launch selected account".to_string()
        };
        if ui.button(label).clicked() {
            actions.push(Action::Launch);
        }
        if ui
            .button("Performance launch (apply FPS, graphics and flags first)")
            .clicked()
        {
            actions.push(Action::PerformanceLaunch);
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
        ui.horizontal(|ui| {
            ui.label("Theme:");
            let mut changed = false;
            changed |= ui
                .radio_value(&mut self.settings.dark_mode, true, "Dark")
                .changed();
            changed |= ui
                .radio_value(&mut self.settings.dark_mode, false, "Light")
                .changed();
            if changed {
                actions.push(Action::ApplyTheme);
            }
        });
        ui.separator();
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
        ui.label("Delay between launches when starting several accounts (milliseconds).");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.delay_text).desired_width(80.0));
            if ui.button("Apply delay").clicked() {
                actions.push(Action::ApplyLaunchDelay);
            }
        });

        ui.separator();
        ui.label("Graphics quality for every Roblox window, 1 (lowest) to 10 (0 removes the override). Restart Roblox to apply.");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.graphics_text).desired_width(80.0));
            if ui.button("Apply graphics").clicked() {
                actions.push(Action::ApplyGraphics);
            }
        });

        ui.separator();
        ui.label("Global limits for every Roblox client. They apply to running clients within a second and to clients that start later. A RAM cap set on the Home tab overrides the global RAM limit for its clients.");
        ui.horizontal(|ui| {
            ui.label("RAM limit per client (MB, 0 = off, minimum 64):");
            ui.add(egui::TextEdit::singleline(&mut self.ram_text).desired_width(80.0));
        });
        ui.horizontal(|ui| {
            ui.label("CPU limit per client (% of all cores, 0 = off):");
            ui.add(egui::TextEdit::singleline(&mut self.cpu_text).desired_width(80.0));
        });
        if ui.button("Save limits").clicked() {
            actions.push(Action::ApplyLimits);
        }
        ui.separator();
        ui.label("CPU sharing: after each launch, set the priority of Roblox windows and spread them across cores. Applies about 20 seconds after launch.");
        if ui
            .checkbox(
                &mut self.settings.cpu_share_on_launch,
                "Share CPU after launch",
            )
            .changed()
        {
            actions.push(Action::ToggleCpuShare);
        }
        if ui
            .checkbox(
                &mut self.settings.cpu_priority_normal,
                "Use Normal priority (off = Below Normal)",
            )
            .changed()
        {
            actions.push(Action::ToggleCpuShare);
        }

        ui.separator();
        ui.label("Gives running Roblox windows less CPU priority. Can make the window you're watching stutter when the PC is busy.");
        if ui
            .button("Set priority of running Roblox windows")
            .clicked()
        {
            actions.push(Action::LowerPriority);
        }

        ui.separator();
        ui.label("Roblox client flags (JSON object), e.g. { \"FlagName\": value }. Applied to every window. Restart Roblox to apply. Flags can be reset by a Roblox update, so reapply after updating.");
        ui.add(
            egui::TextEdit::multiline(&mut self.flags_text)
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .code_editor(),
        );
        ui.label("Fast flags. Checked flags are applied before each launch, and right away when you change them.");
        for (category, name, value) in Self::FAST_FLAGS {
            let mut on = self
                .settings
                .enabled_flags
                .split(',')
                .any(|f| f.trim() == *name);
            if ui
                .checkbox(&mut on, format!("{category}: {name} = {value}"))
                .changed()
            {
                actions.push(Action::SetFlag(name.to_string(), on));
            }
        }
        if ui.button("Save and apply flags").clicked() {
            actions.push(Action::ApplyFlags);
        }

        ui.separator();
        ui.label("Window size for running Roblox windows. Smaller windows use less GPU work. Roblox may refuse sizes below its minimum.");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.resize_width).desired_width(60.0));
            ui.label("x");
            ui.add(egui::TextEdit::singleline(&mut self.resize_height).desired_width(60.0));
            if ui.button("Resize running windows").clicked() {
                actions.push(Action::Resize);
            }
        });

        ui.separator();
        ui.label("Spread running Roblox windows across CPU cores so they don't all compete for the same ones.");
        if ui.button("Spread windows across cores").clicked() {
            actions.push(Action::Spread);
        }

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
        // Keep the Home stats updating even when the mouse is still.
        ctx.request_repaint_after(Duration::from_secs(1));
        ctx.set_visuals(if self.settings.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
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

        let avatars_missing = self.refresh_avatars();
        if avatars_missing {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if self.pending_launch.is_some() || self.pending_login.is_some() {
            ctx.request_repaint();
        }

        let mut actions: Vec<Action> = Vec::new();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Home, "Home"),
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
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.heading("Accounts");
                ui.label(format!(
                    "{} saved, {} selected. Ctrl-click adds or removes, Shift-click selects a range.",
                    self.accounts.len(),
                    self.selected.len()
                ));
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.accounts_list(ui, &mut actions);
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Home => self.home_tab(ui, &mut actions),
            Tab::Accounts => self.accounts_tab(ui, &mut actions),
            Tab::Presets => self.presets_tab(ui, &mut actions),
            Tab::Settings => self.settings_tab(ui, &mut actions),
        });

        for action in actions {
            match action {
                Action::SetTab(tab) => self.tab = tab,
                Action::Select {
                    index,
                    shift,
                    multi,
                } => self.select(index, shift, multi),
                Action::AddAccount => self.add_account(),
                Action::Login => self.start_browser_login(),
                Action::RemoveSelected => self.remove_selected(),
                Action::Launch => self.launch_selected(),
                Action::PerformanceLaunch => self.performance_launch(),
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
                Action::ApplyLaunchDelay => self.apply_launch_delay(),
                Action::ApplyGraphics => self.apply_graphics(),
                Action::ApplyLimits => self.apply_limits(),
                Action::HomeClick { pid, ctrl } => {
                    if ctrl {
                        if !self.home_selected.remove(&pid) {
                            self.home_selected.insert(pid);
                        }
                    } else {
                        self.home_selected.clear();
                        self.home_selected.insert(pid);
                    }
                }
                Action::HomeAdd(pid) => {
                    self.home_selected.insert(pid);
                }
                Action::ClearHomeSelection => self.home_selected.clear(),
                Action::KillClients(pids) => self.kill_clients(&pids),
                Action::ApplyTheme => self.persist_settings(),
                Action::ApplyHomeCap => self.apply_home_cap(),
                Action::SetFlag(name, on) => {
                    self.set_flag(&name, on);
                    self.persist_settings();
                    self.apply_flags();
                }
                Action::RemoveHomeCap => self.set_caps(None),
                Action::LowerPriority => self.lower_priority(),
                Action::ApplyFlags => self.apply_flags(),
                Action::Spread => self.spread(),
                Action::ToggleCpuShare => self.persist_settings(),
                Action::Resize => self.resize_windows(),
            }
        }
    }
}

use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use ksni::{Category, Icon, Status, ToolTip, Tray, TrayService};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

const APP_NAME: &str = "tailscale-ui";
const APP_TITLE: &str = "Tailscale UI";
const APP_VERSION: &str = "0.1.2";
const REFRESH_INTERVAL_SECONDS: u64 = 15;
const ADMIN_CONSOLE_URL: &str = "https://login.tailscale.com/admin";
const LOCAL_WEB_URL: &str = "http://127.0.0.1:8088";
const LOCAL_WEB_LISTEN: &str = "127.0.0.1:8088";
const ICON_SIZE: i32 = 64;

fn xdg_dir(env_name: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(env_name)
        .map(PathBuf::from)
        .map(|path| path.expanduser())
        .unwrap_or(fallback)
}

trait ExpandUser {
    fn expanduser(self) -> Self;
}

impl ExpandUser for PathBuf {
    fn expanduser(self) -> Self {
        if let Some(rest) = self.to_string_lossy().strip_prefix('~') {
            if let Some(home) = env::var_os("HOME") {
                return PathBuf::from(home).join(rest.trim_start_matches('/'));
            }
        }
        self
    }
}

fn desktop_escape(arg: &str) -> String {
    arg.replace('\\', "\\\\").replace(' ', "\\ ")
}

fn command_exists(command: &str) -> bool {
    let path = match env::var_os("PATH") {
        Some(path) => path,
        None => return false,
    };

    for dir in env::split_paths(&path) {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

fn lossless_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

fn argb(a: u8, r: u8, g: u8, b: u8) -> [u8; 4] {
    [a, r, g, b]
}

fn icon_pixels(background: [u8; 4], foreground: [u8; 4]) -> Vec<u8> {
    let mut pixels = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let idx = ((y * ICON_SIZE + x) * 4) as usize;
            let pixel = if x >= 5 && x <= 58 && y >= 5 && y <= 58 {
                background
            } else {
                [0, 0, 0, 0]
            };
            pixels[idx..idx + 4].copy_from_slice(&pixel);
        }
    }

    draw_rounded_rectangle(
        &mut pixels,
        5,
        5,
        59,
        59,
        14,
        background,
        argb(0xff, 0x48, 0x48, 0x48),
    );
    draw_traffic_arrows(&mut pixels, foreground);
    pixels
}

fn draw_rounded_rectangle(
    pixels: &mut [u8],
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    fill: [u8; 4],
    outline: [u8; 4],
) {
    for y in top..=bottom {
        for x in left..=right {
            let inside_rect = x >= left + radius
                && x <= right - radius
                && y >= top
                && y <= bottom
                || y >= top + radius
                    && y <= bottom - radius
                    && x >= left
                    && x <= right;
            let in_corner = {
                let cx = if x < left + radius {
                    left + radius
                } else if x > right - radius {
                    right - radius
                } else {
                    x
                };
                let cy = if y < top + radius {
                    top + radius
                } else if y > bottom - radius {
                    bottom - radius
                } else {
                    y
                };
                let dx = x - cx;
                let dy = y - cy;
                dx * dx + dy * dy <= radius * radius
            };
            if inside_rect || in_corner {
                let idx = ((y * ICON_SIZE + x) * 4) as usize;
                pixels[idx..idx + 4].copy_from_slice(&fill);
            }
        }
    }

    for x in left..=right {
        set_pixel(pixels, x, top, outline);
        set_pixel(pixels, x, bottom, outline);
    }
    for y in top..=bottom {
        set_pixel(pixels, left, y, outline);
        set_pixel(pixels, right, y, outline);
    }
}

fn draw_traffic_arrows(pixels: &mut [u8], fill: [u8; 4]) {
    let up = [
        (16, 29),
        (28, 29),
        (28, 45),
        (35, 45),
        (24, 59),
        (12, 45),
        (19, 45),
    ];
    let down = [
        (48, 35),
        (36, 35),
        (36, 19),
        (29, 19),
        (40, 5),
        (52, 19),
        (45, 19),
    ];
    fill_polygon(pixels, &up, fill);
    fill_polygon(pixels, &down, fill);
}

fn fill_polygon(pixels: &mut [u8], points: &[(i32, i32)], fill: [u8; 4]) {
    let (min_y, max_y) = points.iter().fold((i32::MAX, i32::MIN), |acc, p| {
        (acc.0.min(p.1), acc.1.max(p.1))
    });
    for y in min_y..=max_y {
        let mut intersections = Vec::new();
        for i in 0..points.len() {
            let (x1, y1) = points[i];
            let (x2, y2) = points[(i + 1) % points.len()];
            if (y1 <= y && y < y2) || (y2 <= y && y < y1) {
                let x = x1 as f32
                    + (y - y1) as f32 * (x2 - x1) as f32 / (y2 - y1) as f32;
                intersections.push(x.round() as i32);
            }
        }
        intersections.sort_unstable();
        for pair in intersections.chunks(2) {
            if let [x_start, x_end] = pair {
                for x in *x_start..=*x_end {
                    set_pixel(pixels, x, y, fill);
                }
            }
        }
    }
}

fn set_pixel(pixels: &mut [u8], x: i32, y: i32, rgba: [u8; 4]) {
    if x < 0 || y < 0 || x >= ICON_SIZE || y >= ICON_SIZE {
        return;
    }
    let idx = ((y * ICON_SIZE + x) * 4) as usize;
    pixels[idx..idx + 4].copy_from_slice(&rgba);
}

fn kill_pid(pid: i32) -> io::Result<()> {
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn current_exe_path() -> Result<PathBuf, String> {
    env::current_exe().map_err(|e| format!("failed to resolve current executable: {e}"))
}

fn command_display(args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push("tailscale".to_string());
    parts.extend(args.iter().cloned());
    parts.join(" ")
}

fn command_failure(command: &str, output: &std::process::Output) -> String {
    let stdout = lossless_string(&output.stdout);
    let stderr = lossless_string(&output.stderr);
    let mut parts = vec![format!("Command failed: {command}")];
    if !stdout.is_empty() {
        parts.push(format!("stdout: {stdout}"));
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr: {stderr}"));
    }
    if stdout.is_empty() && stderr.is_empty() {
        parts.push(format!("return code: {:?}", output.status.code()));
    }
    parts.join("\n")
}

fn xdg_open(target: &Path) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to open {}: {e}", target.display()))
}

fn xdg_open_url(url: &str) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to open {url}: {e}"))
}

fn xdg_dir_home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn config_home() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", xdg_dir_home().join(".config"))
}

fn cache_home() -> PathBuf {
    xdg_dir("XDG_CACHE_HOME", xdg_dir_home().join(".cache"))
}

fn config_dir() -> PathBuf {
    config_home().join(APP_NAME)
}

fn cache_dir() -> PathBuf {
    cache_home().join(APP_NAME)
}

fn autostart_dir() -> PathBuf {
    config_home().join("autostart")
}

fn autostart_file() -> PathBuf {
    autostart_dir().join(format!("{APP_NAME}.desktop"))
}

fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

fn local_web_pid_file() -> PathBuf {
    cache_dir().join("local-web-interface.pid")
}

#[derive(Debug, Clone)]
struct ExitNodeChoice {
    node_id: String,
    host_name: String,
    dns_name: String,
    ip: String,
    online: bool,
}

impl ExitNodeChoice {
    fn display_name(&self) -> String {
        let label = if !self.host_name.is_empty() {
            self.host_name.clone()
        } else if !self.dns_name.is_empty() {
            self.dns_name.clone()
        } else {
            self.ip.clone()
        };
        if self.online {
            format!("{label} ({})", self.ip)
        } else {
            format!("{label} ({}, offline)", self.ip)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    preferred_exit_node_id: String,
    preferred_exit_node_name: String,
    preferred_exit_node_ip: String,
    preferred_exit_node_dns: String,
    use_exit_node: bool,
    exit_node_allow_lan_access: bool,
    autostart_enabled: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            preferred_exit_node_id: String::new(),
            preferred_exit_node_name: String::new(),
            preferred_exit_node_ip: String::new(),
            preferred_exit_node_dns: String::new(),
            use_exit_node: true,
            exit_node_allow_lan_access: false,
            autostart_enabled: true,
        }
    }
}

impl AppConfig {
    fn load() -> Self {
        match fs::read_to_string(config_file()) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn save(&self) -> Result<(), String> {
        fs::create_dir_all(config_dir())
            .map_err(|e| format!("failed to create config directory: {e}"))?;
        let contents = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize config: {e}"))?;
        fs::write(config_file(), format!("{contents}\n"))
            .map_err(|e| format!("failed to write config: {e}"))
    }
}

#[derive(Debug, Clone)]
struct TailscaleSnapshot {
    backend_state: String,
    auth_url: String,
    self_host: String,
    self_dns: String,
    self_ips: Vec<String>,
    current_exit_node_id: String,
    current_exit_node_name: String,
    current_exit_node_ip: String,
    peers: Vec<ExitNodeChoice>,
}

impl TailscaleSnapshot {
    fn connected(&self) -> bool {
        self.backend_state == "Running"
    }

    fn login_required(&self) -> bool {
        !self.auth_url.is_empty() && self.backend_state != "Running"
    }

    fn from_json(raw: &Value) -> Result<Self, String> {
        let self_info = raw
            .get("Self")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let peer_map = raw
            .get("Peer")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let current_exit = raw
            .get("ExitNodeStatus")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let current_exit_ips: Vec<String> = current_exit
            .get("TailscaleIPs")
            .and_then(Value::as_array)
            .map(|ips| {
                ips.iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let mut peers = Vec::new();
        for node in peer_map.values().filter_map(Value::as_object) {
            if !node
                .get("ExitNodeOption")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                continue;
            }
            let ips: Vec<String> = node
                .get("TailscaleIPs")
                .and_then(Value::as_array)
                .map(|ips| {
                    ips.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            if ips.is_empty() {
                continue;
            }
            peers.push(ExitNodeChoice {
                node_id: node
                    .get("ID")
                    .and_then(Value::as_i64)
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                host_name: node
                    .get("HostName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                dns_name: node
                    .get("DNSName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim_end_matches('.')
                    .to_string(),
                ip: ips[0].clone(),
                online: node.get("Online").and_then(Value::as_bool).unwrap_or(false),
            });
        }

        peers.sort_by(|a, b| {
            (
                usize::from(!a.online),
                a.host_name.to_lowercase(),
                a.ip.clone(),
            )
                .cmp(&(
                    usize::from(!b.online),
                    b.host_name.to_lowercase(),
                    b.ip.clone(),
                ))
        });

        Self {
            backend_state: raw
                .get("BackendState")
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_string(),
            auth_url: raw
                .get("AuthURL")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            self_host: self_info
                .get("HostName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            self_dns: self_info
                .get("DNSName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_end_matches('.')
                .to_string(),
            self_ips: self_info
                .get("TailscaleIPs")
                .and_then(Value::as_array)
                .map(|ips| {
                    ips.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            current_exit_node_id: current_exit
                .get("ID")
                .and_then(Value::as_i64)
                .map(|v| v.to_string())
                .unwrap_or_default(),
            current_exit_node_name: current_exit
                .get("HostName")
                .or_else(|| current_exit.get("DNSName"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_end_matches('.')
                .to_string(),
            current_exit_node_ip: current_exit_ips.first().cloned().unwrap_or_default(),
            peers,
        }
        .enrich_current_exit_name(peer_map, current_exit_ips)
    }

    fn enrich_current_exit_name(
        mut self,
        peer_map: serde_json::Map<String, Value>,
        current_exit_ips: Vec<String>,
    ) -> Result<Self, String> {
        if !self.current_exit_node_id.is_empty() {
            for node in peer_map.values().filter_map(Value::as_object) {
                if node
                    .get("ID")
                    .and_then(Value::as_i64)
                    .map(|v| v.to_string())
                    .unwrap_or_default()
                    != self.current_exit_node_id
                {
                    continue;
                }
                self.current_exit_node_name = node
                    .get("HostName")
                    .or_else(|| node.get("DNSName"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim_end_matches('.')
                    .to_string();
                if self.current_exit_node_ip.is_empty() {
                    if let Some(ip) = node
                        .get("TailscaleIPs")
                        .and_then(Value::as_array)
                        .and_then(|ips| ips.first())
                        .and_then(Value::as_str)
                    {
                        self.current_exit_node_ip = ip.to_string();
                    }
                }
                break;
            }
        } else if self.current_exit_node_name.is_empty() && !current_exit_ips.is_empty() {
            for node in peer_map.values().filter_map(Value::as_object) {
                let ips: Vec<String> = node
                    .get("TailscaleIPs")
                    .and_then(Value::as_array)
                    .map(|ips| {
                        ips.iter()
                            .filter_map(Value::as_str)
                            .map(ToOwned::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if current_exit_ips
                    .iter()
                    .any(|ip| ips.iter().any(|candidate| candidate == ip))
                {
                    self.current_exit_node_name = node
                        .get("HostName")
                        .or_else(|| node.get("DNSName"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim_end_matches('.')
                        .to_string();
                    if self.current_exit_node_ip.is_empty() {
                        if let Some(ip) = ips.first() {
                            self.current_exit_node_ip = ip.clone();
                        }
                    }
                    break;
                }
            }
        }
        Ok(self)
    }

    fn normalize(value: &str) -> String {
        value.trim().trim_end_matches('.').to_lowercase()
    }

    fn find_preferred_peer(&self, config: &AppConfig) -> Option<ExitNodeChoice> {
        if self.peers.is_empty() {
            return None;
        }

        let candidates = [
            Self::normalize(&config.preferred_exit_node_id),
            Self::normalize(&config.preferred_exit_node_name),
            Self::normalize(&config.preferred_exit_node_dns),
            Self::normalize(&config.preferred_exit_node_ip),
        ];

        self.peers.iter().find_map(|peer| {
            let peer_ids = [
                Self::normalize(&peer.node_id),
                Self::normalize(&peer.host_name),
                Self::normalize(&peer.dns_name),
                Self::normalize(&peer.ip),
            ];
            if candidates.iter().any(|candidate| {
                !candidate.is_empty() && peer_ids.iter().any(|peer_id| peer_id == candidate)
            }) {
                Some(peer.clone())
            } else {
                None
            }
        })
    }
}

#[derive(Debug)]
struct TailscaleTrayApp {
    config: AppConfig,
    snapshot: Option<TailscaleSnapshot>,
    last_message: String,
    error_message: Option<String>,
    current_exe: PathBuf,
    local_web_child: Option<Child>,
}

impl TailscaleTrayApp {
    fn new() -> Result<Self, String> {
        fs::create_dir_all(config_dir())
            .map_err(|e| format!("failed to create config directory: {e}"))?;
        fs::create_dir_all(cache_dir())
            .map_err(|e| format!("failed to create cache directory: {e}"))?;
        fs::create_dir_all(autostart_dir())
            .map_err(|e| format!("failed to create autostart directory: {e}"))?;

        let current_exe = current_exe_path()?;
        let mut app = Self {
            config: AppConfig::load(),
            snapshot: None,
            last_message: String::new(),
            error_message: None,
            current_exe,
            local_web_child: None,
        };
        app.ensure_autostart_file();
        app.refresh_status_sync();
        Ok(app)
    }

    fn report_error(&mut self, message: String) {
        self.error_message = Some(message.clone());
        self.last_message = message;
    }

    fn clear_error(&mut self) {
        self.error_message = None;
    }

    fn save_config(&mut self) {
        if let Err(err) = self.config.save() {
            self.report_error(err);
        }
        self.ensure_autostart_file();
    }

    fn ensure_autostart_file(&mut self) {
        if !self.config.autostart_enabled {
            if let Err(err) = fs::remove_file(autostart_file()) {
                if err.kind() != io::ErrorKind::NotFound {
                    self.report_error(format!("failed to remove autostart file: {err}"));
                }
            }
            return;
        }

        let exec_line = desktop_escape(&self.current_exe.display().to_string());
        let desktop = [
            "[Desktop Entry]",
            "Type=Application",
            &format!("Name={APP_TITLE}"),
            "Comment=Tray controller for Tailscale",
            &format!("Exec={exec_line}"),
            "X-GNOME-Autostart-enabled=true",
            "NoDisplay=true",
            "",
        ]
        .join("\n");

        if let Err(err) = fs::write(autostart_file(), desktop) {
            self.report_error(format!("failed to write autostart file: {err}"));
        }
    }

    fn status_line(&self) -> String {
        if self.error_message.is_some() {
            return "Status: error".to_string();
        }
        match &self.snapshot {
            None => "Status: loading".to_string(),
            Some(snapshot) if snapshot.connected() => {
                format!(
                    "Status: on  Exit: {}",
                    self.current_exit_node_label(snapshot)
                )
            }
            Some(snapshot) if snapshot.login_required() => "Status: login required".to_string(),
            Some(_) => "Status: off".to_string(),
        }
    }

    fn current_exit_node_label(&self, snapshot: &TailscaleSnapshot) -> String {
        let name = snapshot.current_exit_node_name.trim();
        let ip = snapshot.current_exit_node_ip.trim();
        if !name.is_empty() && !ip.is_empty() && name != ip {
            format!("{name} ({ip})")
        } else {
            if !name.is_empty() {
                name.to_string()
            } else if !ip.is_empty() {
                ip.to_string()
            } else {
                "none".to_string()
            }
        }
    }

    fn read_status(&self) -> Result<TailscaleSnapshot, String> {
        let output = Command::new("tailscale")
            .args(["status", "--json"])
            .output()
            .map_err(|e| format!("failed to run tailscale status --json: {e}"))?;
        if !output.status.success() {
            return Err(command_failure(
                &command_display(&vec!["status".to_string(), "--json".to_string()]),
                &output,
            ));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("failed to parse tailscale status json: {e}"))?;
        TailscaleSnapshot::from_json(&value)
    }

    fn preferred_exit_node_to_apply(&self, snapshot: &TailscaleSnapshot) -> Option<ExitNodeChoice> {
        if !snapshot.connected() || !self.config.use_exit_node {
            return None;
        }
        let peer = snapshot.find_preferred_peer(&self.config)?;
        if !peer.online {
            return None;
        }
        if !snapshot.current_exit_node_id.is_empty()
            && snapshot.current_exit_node_id == peer.node_id
        {
            return None;
        }
        Some(peer)
    }

    fn run_tailscale_command(&mut self, args: &[String]) -> Result<String, String> {
        let command = command_display(args);
        let output = Command::new("tailscale")
            .args(args.iter().map(|s| s.as_str()))
            .output()
            .map_err(|e| format!("failed to run {command}: {e}"))?;
        if output.status.success() {
            Ok(lossless_string(&output.stdout))
        } else {
            Err(command_failure(&command, &output))
        }
    }

    fn refresh_status_sync(&mut self) {
        match self.refresh_status_inner() {
            Ok(()) => self.clear_error(),
            Err(err) => self.report_error(err),
        }
    }

    fn refresh_status_inner(&mut self) -> Result<(), String> {
        let snapshot = self.read_status()?;
        self.last_message = self.status_line_for_snapshot(&snapshot);
        if let Some(peer) = self.preferred_exit_node_to_apply(&snapshot) {
            self.last_message = format!("Applying saved exit node: {}", peer.display_name());
            let args = vec![
                "set".to_string(),
                format!("--exit-node={}", peer.ip),
                format!(
                    "--exit-node-allow-lan-access={}",
                    self.config.exit_node_allow_lan_access
                ),
            ];
            self.run_tailscale_command(&args)?;
            let snapshot = self.read_status()?;
            self.snapshot = Some(snapshot);
        } else {
            self.snapshot = Some(snapshot);
        }
        Ok(())
    }

    fn status_line_for_snapshot(&self, snapshot: &TailscaleSnapshot) -> String {
        if snapshot.connected() {
            format!(
                "Tailscale is connected through {}",
                self.current_exit_node_label(snapshot)
            )
        } else if snapshot.login_required() {
            "Tailscale login required".to_string()
        } else {
            "Tailscale is disconnected".to_string()
        }
    }

    fn launch_local_web_interface(&mut self) {
        if self.local_web_interface_running() {
            return;
        }
        match Command::new("tailscale")
            .args(["web", "--listen", LOCAL_WEB_LISTEN])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                let pid = child.id() as i32;
                self.local_web_child = Some(child);
                if let Err(err) = self.write_local_web_pid(pid) {
                    self.report_error(format!("failed to write local web pid: {err}"));
                } else {
                    self.last_message = format!("Started local web interface on {LOCAL_WEB_URL}");
                }
            }
            Err(err) => self.report_error(format!("failed to start local web interface: {err}")),
        }
    }

    fn stop_local_web_interface(&mut self) {
        let pid = match self.local_web_interface_pid() {
            Some(pid) => pid,
            None => {
                self.clear_local_web_pid();
                self.local_web_child = None;
                return;
            }
        };

        if let Some(mut child) = self.local_web_child.take() {
            if child.id() as i32 == pid {
                if let Err(err) = child.kill() {
                    self.report_error(format!("failed to stop local web interface: {err}"));
                } else {
                    let _ = child.wait();
                    self.last_message = "Stopped local web interface".to_string();
                }
            } else {
                self.local_web_child = Some(child);
                if let Err(err) = kill_pid(pid) {
                    self.report_error(format!("failed to stop local web interface: {err}"));
                } else {
                    self.last_message = "Stopped local web interface".to_string();
                }
            }
        } else if let Err(err) = kill_pid(pid) {
            self.report_error(format!("failed to stop local web interface: {err}"));
        } else {
            self.last_message = "Stopped local web interface".to_string();
        }
        self.clear_local_web_pid();
    }

    fn local_web_interface_pid(&self) -> Option<i32> {
        if let Some(child) = &self.local_web_child {
            let pid = child.id() as i32;
            if self.pid_is_running(pid) {
                return Some(pid);
            }
        }
        match fs::read_to_string(local_web_pid_file()) {
            Ok(contents) => match contents.trim().parse::<i32>() {
                Ok(pid) if self.pid_is_running(pid) => Some(pid),
                Ok(pid) => {
                    let _ = fs::remove_file(local_web_pid_file());
                    let _ = pid;
                    None
                }
                Err(_) => None,
            },
            Err(_) => None,
        }
    }

    fn local_web_interface_running(&self) -> bool {
        self.local_web_interface_pid().is_some()
    }

    fn pid_is_running(&self, pid: i32) -> bool {
        let rc = unsafe { libc::kill(pid, 0) };
        if rc == 0 {
            true
        } else {
            io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }

    fn write_local_web_pid(&self, pid: i32) -> Result<(), io::Error> {
        fs::create_dir_all(cache_dir())?;
        fs::write(local_web_pid_file(), format!("{pid}\n"))
    }

    fn clear_local_web_pid(&self) {
        let _ = fs::remove_file(local_web_pid_file());
    }

    fn toggle_local_web_interface(&mut self) {
        if self.local_web_interface_running() {
            self.stop_local_web_interface();
        } else {
            self.launch_local_web_interface();
        }
        self.refresh_status_sync();
    }

    fn open_local_web_interface(&mut self) {
        if self.local_web_interface_running() {
            if let Err(err) = xdg_open_url(LOCAL_WEB_URL) {
                self.report_error(err);
            }
        }
    }

    fn toggle_connection(&mut self) {
        let args = if self
            .snapshot
            .as_ref()
            .map(|s| s.connected())
            .unwrap_or(false)
        {
            vec!["down".to_string()]
        } else {
            vec!["up".to_string()]
        };
        self.last_message = if args[0] == "down" {
            "Disconnecting Tailscale".to_string()
        } else {
            "Connecting Tailscale".to_string()
        };
        if let Err(err) = self.run_tailscale_command(&args) {
            self.report_error(err);
        }
        self.refresh_status_sync();
    }

    fn apply_exit_node(&mut self, peer: ExitNodeChoice) {
        self.config.preferred_exit_node_id = peer.node_id.clone();
        self.config.preferred_exit_node_name = peer.host_name.clone();
        self.config.preferred_exit_node_ip = peer.ip.clone();
        self.config.preferred_exit_node_dns = peer.dns_name.clone();
        self.config.use_exit_node = true;
        self.save_config();

        let args = vec![
            "set".to_string(),
            format!("--exit-node={}", peer.ip),
            format!(
                "--exit-node-allow-lan-access={}",
                self.config.exit_node_allow_lan_access
            ),
        ];
        self.last_message = format!("Using exit node: {}", peer.display_name());
        if let Err(err) = self.run_tailscale_command(&args) {
            self.report_error(err);
        }
        self.refresh_status_sync();
    }

    fn clear_exit_node(&mut self) {
        self.config.use_exit_node = false;
        self.save_config();
        if !self
            .snapshot
            .as_ref()
            .map(|s| s.connected())
            .unwrap_or(false)
        {
            self.refresh_status_sync();
            return;
        }

        let args = vec!["set".to_string(), "--exit-node=".to_string()];
        self.last_message = "Clearing exit node".to_string();
        if let Err(err) = self.run_tailscale_command(&args) {
            self.report_error(err);
        }
        self.refresh_status_sync();
    }

    fn toggle_use_exit_node(&mut self, enabled: bool) {
        self.config.use_exit_node = enabled;
        self.save_config();

        let snapshot = self.snapshot.clone();
        if enabled {
            if let Some(snapshot) = snapshot.as_ref() {
                if snapshot.connected() {
                    if let Some(peer) = snapshot.find_preferred_peer(&self.config) {
                        if peer.online {
                            self.apply_exit_node(peer);
                            return;
                        }
                    }
                }
            }
        } else {
            self.clear_exit_node();
            return;
        }
        self.refresh_status_sync();
    }

    fn toggle_lan_access(&mut self, enabled: bool) {
        self.config.exit_node_allow_lan_access = enabled;
        self.save_config();
        let snapshot = self.snapshot.clone();
        if let Some(snapshot) = snapshot.as_ref() {
            if snapshot.connected() && self.config.use_exit_node {
                if let Some(peer) = snapshot.find_preferred_peer(&self.config) {
                    self.apply_exit_node(peer);
                    return;
                }
            }
        }
        self.refresh_status_sync();
    }

    fn toggle_autostart(&mut self, enabled: bool) {
        self.config.autostart_enabled = enabled;
        self.save_config();
        self.refresh_status_sync();
    }

    fn open_folder(&mut self, path: &Path) {
        if let Err(err) = xdg_open(path) {
            self.report_error(err);
        }
    }

    fn cleanup_local_web_interface(&mut self) {
        if self.local_web_interface_running() {
            self.stop_local_web_interface();
        }
    }

    fn quit(&mut self) {
        self.cleanup_local_web_interface();
        std::process::exit(0);
    }

    fn rebuild_menu(&self) -> Vec<MenuItem<Self>> {
        let mut items = Vec::new();

        let mut status_item = StandardItem::default();
        status_item.label = self.status_line();
        status_item.enabled = false;
        items.push(status_item.into());

        if let Some(snapshot) = &self.snapshot {
            if snapshot.connected() {
                let mut connected_item = StandardItem::default();
                connected_item.label = format!(
                    "Connected through: {}",
                    self.current_exit_node_label(snapshot)
                );
                connected_item.enabled = false;
                items.push(connected_item.into());
            } else if snapshot.login_required() {
                let mut login_item = StandardItem::default();
                login_item.label = "Login required".to_string();
                login_item.enabled = false;
                items.push(login_item.into());
            }
        }

        items.push(MenuItem::Separator);

        let mut action_item = StandardItem::default();
        action_item.label = if self
            .snapshot
            .as_ref()
            .map(|s| s.connected())
            .unwrap_or(false)
        {
            "Disconnect Tailscale".to_string()
        } else {
            "Connect Tailscale".to_string()
        };
        action_item.activate = Box::new(|this: &mut Self| this.toggle_connection());
        items.push(action_item.into());

        let mut local_web_status = StandardItem::default();
        local_web_status.label = if self.local_web_interface_running() {
            "Web interface: running".to_string()
        } else {
            "Web interface: stopped".to_string()
        };
        local_web_status.enabled = false;

        let mut web_toggle = StandardItem::default();
        web_toggle.label = if self.local_web_interface_running() {
            "Stop local web interface".to_string()
        } else {
            "Run local web interface".to_string()
        };
        web_toggle.activate = Box::new(|this: &mut Self| this.toggle_local_web_interface());

        let mut open_web = StandardItem::default();
        open_web.label = "Open local web interface".to_string();
        open_web.enabled = self.local_web_interface_running();
        open_web.activate = Box::new(|this: &mut Self| this.open_local_web_interface());

        items.push(
            SubMenu {
                label: "Local web interface".to_string(),
                submenu: vec![local_web_status.into(), web_toggle.into(), open_web.into()],
                ..Default::default()
            }
            .into(),
        );

        let mut exit_node_items = Vec::new();
        let mut exit_info = StandardItem::default();
        exit_info.label = if let Some(snapshot) = &self.snapshot {
            if snapshot.connected() {
                format!(
                    "Connected through: {}",
                    self.current_exit_node_label(snapshot)
                )
            } else {
                "Exit node: off".to_string()
            }
        } else {
            "Exit node: off".to_string()
        };
        exit_info.enabled = false;
        exit_node_items.push(exit_info.into());

        let mut use_exit_node = CheckmarkItem::default();
        use_exit_node.label = "Use saved exit node".to_string();
        use_exit_node.checked = self.config.use_exit_node;
        use_exit_node.activate = Box::new(|this: &mut Self| {
            let new_value = !this.config.use_exit_node;
            this.toggle_use_exit_node(new_value);
        });
        exit_node_items.push(use_exit_node.into());

        let mut lan_access = CheckmarkItem::default();
        lan_access.label = "Allow LAN access via exit node".to_string();
        lan_access.checked = self.config.exit_node_allow_lan_access;
        lan_access.enabled = self
            .snapshot
            .as_ref()
            .map(|s| s.connected())
            .unwrap_or(false);
        lan_access.activate = Box::new(|this: &mut Self| {
            let new_value = !this.config.exit_node_allow_lan_access;
            this.toggle_lan_access(new_value);
        });
        exit_node_items.push(lan_access.into());

        let mut choose_submenu_items = Vec::new();
        if let Some(snapshot) = &self.snapshot {
            if !snapshot.peers.is_empty() {
                let current_choice = snapshot.find_preferred_peer(&self.config);
                for peer in snapshot.peers.iter().cloned() {
                    let mut item = StandardItem::default();
                    item.label = peer.display_name();
                    if !peer.online {
                        item.enabled = false;
                    }
                    if snapshot.current_exit_node_id == peer.node_id {
                        item.label = format!("{}  [current]", item.label);
                    }
                    if current_choice
                        .as_ref()
                        .map(|choice| choice.node_id == peer.node_id)
                        .unwrap_or(false)
                    {
                        item.label = format!("{}  [saved]", item.label);
                    }
                    item.activate =
                        Box::new(move |this: &mut Self| this.apply_exit_node(peer.clone()));
                    choose_submenu_items.push(item.into());
                }
                choose_submenu_items.push(MenuItem::Separator);
            } else {
                let mut none = StandardItem::default();
                none.label = "No exit nodes found".to_string();
                none.enabled = false;
                choose_submenu_items.push(none.into());
            }
        } else {
            let mut none = StandardItem::default();
            none.label = "No exit nodes found".to_string();
            none.enabled = false;
            choose_submenu_items.push(none.into());
        }

        let mut clear = StandardItem::default();
        clear.label = "Clear exit node".to_string();
        clear.activate = Box::new(|this: &mut Self| this.clear_exit_node());
        choose_submenu_items.push(clear.into());

        exit_node_items.push(
            SubMenu {
                label: "Choose exit node".to_string(),
                submenu: choose_submenu_items,
                ..Default::default()
            }
            .into(),
        );
        items.push(
            SubMenu {
                label: "Exit node".to_string(),
                submenu: exit_node_items,
                ..Default::default()
            }
            .into(),
        );

        let mut admin_items = Vec::new();
        if let Some(snapshot) = &self.snapshot {
            if !snapshot.auth_url.is_empty() {
                let auth_url = snapshot.auth_url.clone();
                let mut login_item = StandardItem::default();
                login_item.label = "Open login page".to_string();
                login_item.activate = Box::new(move |this: &mut Self| {
                    if let Err(err) = xdg_open_url(&auth_url) {
                        this.report_error(err);
                    }
                });
                admin_items.push(login_item.into());
            }
        }

        let mut admin_console = StandardItem::default();
        admin_console.label = "Open Tailscale admin console".to_string();
        admin_console.activate = Box::new(|this: &mut Self| {
            if let Err(err) = xdg_open_url(ADMIN_CONSOLE_URL) {
                this.report_error(err);
            }
        });
        admin_items.push(admin_console.into());

        let mut autostart = CheckmarkItem::default();
        autostart.label = "Start automatically on login".to_string();
        autostart.checked = self.config.autostart_enabled;
        autostart.activate = Box::new(|this: &mut Self| {
            let new_value = !this.config.autostart_enabled;
            this.toggle_autostart(new_value);
        });
        admin_items.push(autostart.into());

        items.push(
            SubMenu {
                label: "Admin".to_string(),
                submenu: admin_items,
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        let mut refresh = StandardItem::default();
        refresh.label = "Refresh now".to_string();
        refresh.activate = Box::new(|this: &mut Self| this.refresh_status_sync());
        items.push(refresh.into());

        let mut open_config = StandardItem::default();
        open_config.label = "Open config folder".to_string();
        open_config.activate = Box::new(|this: &mut Self| this.open_folder(&config_dir()));
        items.push(open_config.into());

        let mut quit = StandardItem::default();
        quit.label = "Quit".to_string();
        quit.activate = Box::new(|this: &mut Self| this.quit());
        items.push(quit.into());

        items
    }
}

impl Tray for TailscaleTrayApp {
    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn id(&self) -> String {
        APP_NAME.to_string()
    }

    fn title(&self) -> String {
        format!("{APP_TITLE} {APP_VERSION}")
    }

    fn status(&self) -> Status {
        if self.error_message.is_some() {
            Status::NeedsAttention
        } else {
            Status::Active
        }
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let (bg, fg) = if let Some(snapshot) = &self.snapshot {
            if snapshot.connected() {
                (argb(0xff, 0x00, 0x00, 0x00), argb(0xff, 0x44, 0xff, 0x44))
            } else if snapshot.login_required() {
                (argb(0xff, 0x00, 0x00, 0x00), argb(0xff, 0xff, 0xf2, 0x66))
            } else {
                (argb(0xff, 0x00, 0x00, 0x00), argb(0xff, 0xf0, 0xf0, 0xf0))
            }
        } else {
            (argb(0xff, 0x00, 0x00, 0x00), argb(0xff, 0xf0, 0xf0, 0xf0))
        };

        vec![Icon {
            width: ICON_SIZE,
            height: ICON_SIZE,
            data: icon_pixels(bg, fg),
        }]
    }

    fn attention_icon_pixmap(&self) -> Vec<Icon> {
        if self.error_message.is_none() {
            return Vec::new();
        }

        vec![Icon {
            width: ICON_SIZE,
            height: ICON_SIZE,
            data: icon_pixels(argb(0xff, 0x5a, 0x00, 0x00), argb(0xff, 0xff, 0x66, 0x66)),
        }]
    }

    fn attention_icon_name(&self) -> String {
        if self.error_message.is_some() {
            "dialog-error".to_string()
        } else {
            String::new()
        }
    }

    fn tool_tip(&self) -> ToolTip {
        let description = if let Some(snapshot) = &self.snapshot {
            let mut lines = vec![self.status_line()];
            lines.push(format!(
                "Self: {} {}",
                snapshot.self_host, snapshot.self_dns
            ));
            if !snapshot.self_ips.is_empty() {
                lines.push(format!("IPs: {}", snapshot.self_ips.join(", ")));
            }
            lines.push(format!(
                "Local web: {}",
                if self.local_web_interface_running() {
                    "running"
                } else {
                    "stopped"
                }
            ));
            if !self.last_message.is_empty() {
                lines.push(format!("Last: {}", self.last_message));
            }
            lines.join("\n")
        } else if !self.last_message.is_empty() {
            format!("{}\nLast: {}", self.status_line(), self.last_message)
        } else {
            self.status_line()
        };

        ToolTip {
            icon_name: self.icon_name(),
            icon_pixmap: Vec::new(),
            title: APP_TITLE.to_string(),
            description,
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        self.rebuild_menu()
    }
}

fn main() -> Result<(), String> {
    if !command_exists("tailscale") {
        return Err("tailscale command not found in PATH".to_string());
    }

    let app = TailscaleTrayApp::new().map_err(|e| format!("Failed to start tray app: {e}"))?;
    let service = TrayService::new(app);
    let handle = service.handle();
    service.spawn();

    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(REFRESH_INTERVAL_SECONDS));
        handle.update(|tray: &mut TailscaleTrayApp| {
            tray.refresh_status_sync();
        });
    });

    loop {
        thread::park();
    }
}

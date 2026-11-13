//! Docker containers for the Docker widget, read from Docker's local API pipe (`\\.\pipe\docker_engine`, the one
//! Docker Desktop and the `docker` CLI use). No `docker` process is started: one small HTTP request over the pipe.
//! When the pipe isn't there, Docker isn't running and the widget stays hidden.

use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::mpsc::channel;
use std::time::Duration;

const PIPE: &str = r"\\.\pipe\docker_engine";
/// A request that takes longer is given up (Docker starting up or hanging); the bar never waits on it.
const TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// Docker's state: running, exited, paused, restarting, created, dead.
    pub state: String,
    /// Docker's human text, e.g. "Up 3 hours" / "Exited (0) 2 days ago".
    pub status: String,
    /// The compose project it belongs to, if any.
    pub project: Option<String>,
    /// Published ports, e.g. "8080→80".
    pub ports: Vec<String>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Docker {
    pub running: usize,
    pub containers: Vec<Container>,
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Start,
    Stop,
    Restart,
}

/// Docker's answer to GET /containers/json (only the fields used).
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Raw {
    id: String,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    image: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    ports: Vec<RawPort>,
    #[serde(default)]
    labels: Option<std::collections::HashMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPort {
    #[serde(default)]
    private_port: u16,
    #[serde(default)]
    public_port: Option<u16>,
}

/// Only one request at a time: a hung Docker must not pile up waiting threads.
static BUSY: AtomicBool = AtomicBool::new(false);

/// One HTTP/1.0 request over the pipe (Docker closes the connection after the answer). (status, body).
fn request(method: &'static str, path: String) -> Result<(u16, Vec<u8>), String> {
    if BUSY.swap(true, SeqCst) {
        return Err("Docker is still answering the last request".into());
    }
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let result = (|| {
            let mut pipe = OpenOptions::new().read(true).write(true).open(PIPE).map_err(|e| e.to_string())?;
            write!(pipe, "{method} {path} HTTP/1.0\r\nHost: docker\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .map_err(|e| e.to_string())?;
            let mut raw = Vec::new();
            pipe.read_to_end(&mut raw).map_err(|e| e.to_string())?;
            parse_response(&raw).ok_or_else(|| "unreadable answer from Docker".to_string())
        })();
        BUSY.store(false, SeqCst);
        let _ = tx.send(result);
    });
    rx.recv_timeout(TIMEOUT).unwrap_or_else(|_| Err("Docker didn't answer".into()))
}

/// Splits an HTTP response into its status code and body.
fn parse_response(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..end]).ok()?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, raw[end + 4..].to_vec()))
}

/// The containers (running first), or None when Docker isn't running.
pub fn read() -> Option<Docker> {
    let (status, body) = request("GET", "/containers/json?all=1".into()).ok()?;
    if status != 200 {
        return None;
    }
    from_json(&body)
}

fn from_json(body: &[u8]) -> Option<Docker> {
    let raw: Vec<Raw> = serde_json::from_slice(body).ok()?;
    let mut containers: Vec<Container> = raw
        .into_iter()
        .map(|r| {
            let mut ports: Vec<String> = r
                .ports
                .iter()
                .filter_map(|p| p.public_port.map(|public| format!("{public}→{}", p.private_port)))
                .collect();
            ports.sort();
            ports.dedup(); // Docker lists IPv4 and IPv6 bindings separately
            Container {
                id: r.id.chars().take(12).collect(),
                name: r.names.first().map(|n| n.trim_start_matches('/').to_string()).unwrap_or_default(),
                image: r.image,
                state: r.state,
                status: r.status,
                project: r.labels.and_then(|l| l.get("com.docker.compose.project").cloned()),
                ports,
            }
        })
        .collect();
    containers.sort_by(|a, b| {
        (a.state != "running", &a.project, &a.name).cmp(&(b.state != "running", &b.project, &b.name))
    });
    Some(Docker { running: containers.iter().filter(|c| c.state == "running").count(), containers })
}

/// Starts, stops or restarts a container (by its id from `read`).
pub fn act(id: &str, action: Action) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("not a container id".into());
    }
    let verb = match action {
        Action::Start => "start",
        Action::Stop => "stop",
        Action::Restart => "restart",
    };
    match request("POST", format!("/containers/{id}/{verb}"))? {
        // 304: already started / stopped.
        (204 | 304, _) => Ok(()),
        (status, body) => {
            let message = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
                .unwrap_or_else(|| format!("HTTP {status}"));
            crate::win::log(&format!("docker {verb} failed: {message}"));
            Err(message)
        }
    }
}

/// Opens Docker Desktop (per-user or machine-wide install).
pub fn open_desktop() {
    let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_default();
    let program_files = std::env::var_os("ProgramFiles").map(std::path::PathBuf::from).unwrap_or_default();
    let places = [
        local.join("Programs").join("DockerDesktop").join("Docker Desktop.exe"),
        program_files.join("Docker").join("Docker").join("Docker Desktop.exe"),
    ];
    if let Some(exe) = places.iter().find(|p| p.exists()) {
        crate::win::open(&exe.to_string_lossy());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_containers_from_the_api() {
        let body = br#"[
            {"Id":"aaaaaaaaaaaaaaaa","Names":["/web"],"Image":"nginx","State":"running","Status":"Up 3 hours",
             "Ports":[{"PrivatePort":80,"PublicPort":8080,"Type":"tcp","IP":"0.0.0.0"},{"PrivatePort":80,"PublicPort":8080,"Type":"tcp","IP":"::"},{"PrivatePort":443,"Type":"tcp"}],
             "Labels":{"com.docker.compose.project":"site"}},
            {"Id":"bbbbbbbbbbbbbbbb","Names":["/db"],"Image":"postgres","State":"exited","Status":"Exited (0) 2 days ago","Ports":[],"Labels":{}},
            {"Id":"cccccccccccccccc","Names":["/api"],"Image":"node","State":"running","Status":"Up 1 minute","Ports":[],"Labels":null}
        ]"#;
        let d = from_json(body).unwrap();
        assert_eq!(d.running, 2);
        assert_eq!(d.containers.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["api", "web", "db"], "running first");
        let web = &d.containers[1];
        assert_eq!((web.id.as_str(), web.ports.clone(), web.project.as_deref()), ("aaaaaaaaaaaa", vec!["8080→80".to_string()], Some("site")));
        assert!(from_json(b"not json").is_none());
    }

    #[test]
    fn splits_http_answers() {
        let (status, body) = parse_response(b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n[]").unwrap();
        assert_eq!((status, body.as_slice()), (200, b"[]".as_slice()));
        assert_eq!(parse_response(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap().0, 204);
        assert!(parse_response(b"garbage").is_none());
    }

    #[test]
    fn only_container_ids_are_sent() {
        assert!(act("../../images", Action::Stop).is_err());
        assert!(act("", Action::Start).is_err());
    }
}

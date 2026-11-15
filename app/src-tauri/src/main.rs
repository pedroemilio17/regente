//! Desktop window for the independent local daemon.
use std::{
    env, fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

fn port() -> u16 {
    let home = env::var_os("REGENTE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/regente")));
    env::var("REGENTE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            let config = fs::read_to_string(home?.join("config.json")).ok()?;
            let value: serde_json::Value = serde_json::from_str(&config).ok()?;
            value
                .get("port")?
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
        })
        .unwrap_or(7717)
}

fn request(port: u16, path: &str) -> io::Result<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn daemon_ready(port: u16) -> bool {
    let Ok(response) = request(port, "/api/v1/health") else {
        return false;
    };
    let Some((headers, body)) = response.split_once("\r\n\r\n") else {
        return false;
    };
    headers.starts_with("HTTP/1.1 200 ")
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .is_some_and(|v| v["ok"] == true && v["api_version"] == "1")
}

fn start_daemon(app: &tauri::App, port: u16) -> io::Result<()> {
    if daemon_ready(port) {
        return Ok(());
    }
    let resources = app.path().resource_dir().map_err(io::Error::other)?;
    let sibling = env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("diretório do aplicativo indisponível"))?
        .join("regented");
    let executable = [sibling, resources.join("regented")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| io::Error::other("binário regented não encontrado no pacote"))?;
    let binary_dir = executable
        .parent()
        .ok_or_else(|| io::Error::other("diretório dos binários indisponível"))?;
    let mut paths = vec![binary_dir.to_path_buf()];
    if let Some(path) = env::var_os("PATH") {
        paths.extend(env::split_paths(&path));
    }
    let path = env::join_paths(paths).map_err(io::Error::other)?;
    // Dropping Child does not terminate it: tasks survive closing the window.
    Command::new(executable)
        .env("PATH", path)
        .arg("serve")
        .env("REGENTE_PORT", port.to_string())
        .env("REGENTE_UI_DIR", resources.join("ui"))
        .env("REGENTE_FIXTURES", resources.join("fixtures"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if daemon_ready(port) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(io::Error::other(
        "daemon não iniciou; verifique se a porta está ocupada",
    ))
}

fn main() -> ExitCode {
    let result = tauri::Builder::default()
        .setup(|app| {
            let port = port();
            start_daemon(app, port)?;
            // A daemon already running must have its own UI configured.
            let response = request(port, "/")?;
            if !response.starts_with("HTTP/1.1 200 ") || !response.to_ascii_lowercase().contains("text/html") {
                return Err(io::Error::other("o daemon ativo não está servindo a interface; configure REGENTE_UI_DIR e reinicie-o").into());
            }
            let url = format!("http://127.0.0.1:{port}/").parse()?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Regente").inner_size(1280.0, 800.0).min_inner_size(900.0, 600.0).build()?;
            Ok(())
        })
        .run(tauri::generate_context!());
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Regente: {error}");
            ExitCode::FAILURE
        }
    }
}

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

pub struct CoreState {
    pub child: Mutex<Option<Child>>,
    pub port_mappings: Mutex<Vec<PortMapping>>,
    pub controller_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub port: u16,
    pub node_name: String,
    pub node_type: String,
    pub server: String,
    pub proxy_type: String, // "mixed" or "socks5"
    pub listen_ip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub port_count: usize,
    pub controller_port: u16,
    pub port_mappings: Vec<PortMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartPoolRequest {
    pub raw_proxies: Vec<serde_yaml::Value>,
    pub start_port: u16,
    pub proxy_type: String, // "mixed" or "socks"
    pub listen_ip: String,  // "127.0.0.1" or "0.0.0.0"
}

pub fn is_port_available(ip: &str, port: u16) -> bool {
    TcpListener::bind((ip, port)).is_ok()
}

pub fn find_available_port(ip: &str, mut port: u16) -> u16 {
    while port < 65535 {
        if is_port_available(ip, port) {
            return port;
        }
        port += 1;
    }
    port
}

pub fn locate_mihomo_binary(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut names = Vec::new();
    if cfg!(windows) {
        names.push("mihomo-x86_64-pc-windows-msvc.exe");
        names.push("mihomo.exe");
    } else if std::env::consts::ARCH == "aarch64" {
        names.push("mihomo-aarch64-apple-darwin");
        names.push("mihomo-darwin-arm64");
        names.push("mihomo");
        names.push("mihomo-x86_64-apple-darwin");
        names.push("mihomo-darwin-amd64");
    } else {
        names.push("mihomo-x86_64-apple-darwin");
        names.push("mihomo-darwin-amd64");
        names.push("mihomo");
        names.push("mihomo-aarch64-apple-darwin");
        names.push("mihomo-darwin-arm64");
    }

    let search_dirs = [
        PathBuf::from("src-tauri/bin"),
        PathBuf::from("bin"),
        app.path().resource_dir().unwrap_or_default().join("bin"),
        app.path().resource_dir().unwrap_or_default(),
        std::env::current_exe().map(|p| p.parent().unwrap().to_path_buf()).unwrap_or_default(),
        std::env::current_exe().map(|p| p.parent().unwrap().join("bin")).unwrap_or_default(),
    ];

    let mut found_source: Option<PathBuf> = None;
    for dir in &search_dirs {
        for name in &names {
            let path = dir.join(name);
            if path.is_file() {
                found_source = Some(path);
                break;
            }
        }
        if found_source.is_some() {
            break;
        }
    }

    let source = match found_source {
        Some(s) => s,
        None => return Err("未找到 Mihomo 内核二进制文件，请确认已放置在 bin 目录中".to_string()),
    };

    // Ensure the binary is copied to app_data_dir/bin so it has full execute permissions
    // and is never blocked by read-only DMG filesystems or macOS Gatekeeper restrictions
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用目录失败: {}", e))?;
    let bin_dir = app_data_dir.join("bin");
    let _ = std::fs::create_dir_all(&bin_dir);

    let target_bin_name = if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    };
    let target_bin = bin_dir.join(target_bin_name);

    let need_copy = if target_bin.is_file() {
        let src_len = std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0);
        let dst_len = std::fs::metadata(&target_bin).map(|m| m.len()).unwrap_or(0);
        src_len != dst_len || src_len == 0
    } else {
        true
    };

    if need_copy {
        let _ = std::fs::copy(&source, &target_bin);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&target_bin) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&target_bin, perms);
        }

        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&target_bin)
                .output();
        }
    }

    if target_bin.is_file() {
        Ok(target_bin)
    } else {
        Ok(source)
    }
}

pub fn generate_config(
    proxies: &[serde_yaml::Value],
    start_port: u16,
    proxy_type: &str,
    listen_ip: &str,
    controller_port: u16,
) -> (String, Vec<PortMapping>) {
    let mut listeners = Vec::new();
    let mut mappings = Vec::new();
    let mut current_port = start_port;

    for p in proxies {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
        let node_type = p.get("type").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
        let server = p.get("server").and_then(|v| v.as_str()).unwrap_or("").to_string();

        current_port = find_available_port(listen_ip, current_port);

        let mut listener = serde_yaml::Mapping::new();
        listener.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String(format!("listener-{}", current_port)),
        );
        listener.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::String(proxy_type.to_string()),
        );
        listener.insert(
            serde_yaml::Value::String("port".to_string()),
            serde_yaml::Value::Number(current_port.into()),
        );
        listener.insert(
            serde_yaml::Value::String("listen".to_string()),
            serde_yaml::Value::String(listen_ip.to_string()),
        );
        listener.insert(
            serde_yaml::Value::String("proxy".to_string()),
            serde_yaml::Value::String(name.clone()),
        );

        listeners.push(serde_yaml::Value::Mapping(listener));

        mappings.push(PortMapping {
            port: current_port,
            node_name: name,
            node_type,
            server,
            proxy_type: proxy_type.to_string(),
            listen_ip: listen_ip.to_string(),
        });

        current_port += 1;
    }

    let mut root = serde_yaml::Mapping::new();
    root.insert(serde_yaml::Value::String("mode".to_string()), serde_yaml::Value::String("rule".to_string()));
    root.insert(serde_yaml::Value::String("log-level".to_string()), serde_yaml::Value::String("warning".to_string()));
    root.insert(serde_yaml::Value::String("ipv6".to_string()), serde_yaml::Value::Bool(false));
    root.insert(
        serde_yaml::Value::String("external-controller".to_string()),
        serde_yaml::Value::String(format!("127.0.0.1:{}", controller_port)),
    );
    root.insert(serde_yaml::Value::String("secret".to_string()), serde_yaml::Value::String("".to_string()));

    let mut dns = serde_yaml::Mapping::new();
    dns.insert(serde_yaml::Value::String("enable".to_string()), serde_yaml::Value::Bool(false));
    root.insert(serde_yaml::Value::String("dns".to_string()), serde_yaml::Value::Mapping(dns));

    root.insert(
        serde_yaml::Value::String("proxies".to_string()),
        serde_yaml::Value::Sequence(proxies.to_vec()),
    );
    root.insert(
        serde_yaml::Value::String("listeners".to_string()),
        serde_yaml::Value::Sequence(listeners),
    );

    let config_yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(root)).unwrap_or_default();
    (config_yaml, mappings)
}

pub fn start_mihomo_instance(
    app: &tauri::AppHandle,
    state: &CoreState,
    req: StartPoolRequest,
) -> Result<PoolStatus, String> {
    // 1. If currently running, stop first
    stop_mihomo_instance(state)?;

    if req.raw_proxies.is_empty() {
        return Err("节点列表为空，无法启动代理池".to_string());
    }

    let mihomo_bin = locate_mihomo_binary(app)?;

    // 2. Prepare app config directory
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用目录失败: {}", e))?;
    std::fs::create_dir_all(&app_data_dir).map_err(|e| format!("创建数据目录失败: {}", e))?;

    let config_path = app_data_dir.join("mihomo_generated.yaml");
    let (config_yaml, mappings) = generate_config(
        &req.raw_proxies,
        req.start_port,
        &req.proxy_type,
        &req.listen_ip,
        state.controller_port,
    );

    std::fs::write(&config_path, config_yaml).map_err(|e| format!("写入配置文件失败: {}", e))?;

    // Pre-validate config before launch
    let check = Command::new(&mihomo_bin)
        .arg("-t")
        .arg("-f")
        .arg(&config_path)
        .output();
    if let Ok(out) = check {
        if !out.status.success() {
            let err_text = String::from_utf8_lossy(&out.stderr);
            return Err(format!("代理池配置校验失败: {}", err_text.trim()));
        }
    }

    // 3. Spawn child process
    let mut cmd = Command::new(&mihomo_bin);
    cmd.arg("-d")
        .arg(&app_data_dir)
        .arg("-f")
        .arg(&config_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("启动 Mihomo 进程失败: {}", e))?;

    // Wait until controller is ready or child exits
    let mut ready = false;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap_or_default();
    let version_url = format!("http://127.0.0.1:{}/version", state.controller_port);

    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(Some(status)) = child.try_wait() {
            let mut stderr_msg = String::new();
            if let Some(mut err_reader) = child.stderr.take() {
                use std::io::Read;
                let _ = err_reader.read_to_string(&mut stderr_msg);
            }
            return Err(format!("Mihomo 启动失败 ({}): {}", status, stderr_msg.trim()));
        }

        let resp_ok = tauri::async_runtime::block_on(async {
            client.get(&version_url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
        });
        if resp_ok {
            ready = true;
            break;
        }
    }

    if !ready {
        let _ = child.kill();
        return Err("Mihomo 启动超时，控制端口未能就绪".to_string());
    }

    let pid = child.id();
    {
        let mut child_lock = state.child.lock().unwrap();
        *child_lock = Some(child);

        let mut mappings_lock = state.port_mappings.lock().unwrap();
        *mappings_lock = mappings.clone();
    }

    Ok(PoolStatus {
        running: true,
        pid: Some(pid),
        port_count: mappings.len(),
        controller_port: state.controller_port,
        port_mappings: mappings,
    })
}

pub fn stop_mihomo_instance(state: &CoreState) -> Result<(), String> {
    let mut child_lock = state.child.lock().unwrap();
    if let Some(mut child) = child_lock.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let mut mappings_lock = state.port_mappings.lock().unwrap();
    mappings_lock.clear();
    Ok(())
}

pub fn get_pool_status(state: &CoreState) -> PoolStatus {
    let mut child_lock = state.child.lock().unwrap();
    let is_running = if let Some(ref mut child) = *child_lock {
        match child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    } else {
        false
    };

    let pid = if is_running {
        child_lock.as_ref().map(|c| c.id())
    } else {
        None
    };

    let mappings_lock = state.port_mappings.lock().unwrap();

    PoolStatus {
        running: is_running,
        pid,
        port_count: mappings_lock.len(),
        controller_port: state.controller_port,
        port_mappings: mappings_lock.clone(),
    }
}

pub async fn test_node_latency(controller_port: u16, node_name: &str) -> Result<i32, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;

    let encoded_name = urlencoding::encode(node_name);

    #[derive(Deserialize)]
    struct DelayResp {
        delay: Option<i32>,
    }

    // 1. First attempt with Google gstatic 204
    let url_gstatic = format!(
        "http://127.0.0.1:{}/proxies/{}/delay?url=https://www.gstatic.com/generate_204&timeout=4500",
        controller_port, encoded_name
    );
    if let Ok(resp) = client.get(&url_gstatic).send().await {
        if resp.status().is_success() {
            if let Ok(data) = resp.json::<DelayResp>().await {
                if let Some(d) = data.delay {
                    if d >= 0 {
                        return Ok(d);
                    }
                }
            }
        }
    }

    // 2. Fallback attempt with Cloudflare 204
    let url_cf = format!(
        "http://127.0.0.1:{}/proxies/{}/delay?url=http://cp.cloudflare.com/generate_204&timeout=4500",
        controller_port, encoded_name
    );
    if let Ok(resp) = client.get(&url_cf).send().await {
        if resp.status().is_success() {
            if let Ok(data) = resp.json::<DelayResp>().await {
                if let Some(d) = data.delay {
                    if d >= 0 {
                        return Ok(d);
                    }
                }
            }
        }
    }

    Ok(-1)
}

pub async fn batch_test_latencies(
    controller_port: u16,
    nodes: Vec<String>,
) -> HashMap<String, i32> {
    use tokio::sync::Semaphore;
    use std::sync::Arc;

    let semaphore = Arc::new(Semaphore::new(10));
    let mut tasks = Vec::new();

    for name in nodes {
        let sem = semaphore.clone();
        let task = tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let delay = test_node_latency(controller_port, &name).await.unwrap_or(-1);
            (name, delay)
        });
        tasks.push(task);
    }

    let mut results = HashMap::new();
    for t in tasks {
        if let Ok((name, delay)) = t.await {
            results.insert(name, delay);
        }
    }
    results
}

pub async fn test_nodes_flexible(
    app: &tauri::AppHandle,
    state: &CoreState,
    proxies: Vec<serde_yaml::Value>,
    target_nodes: Vec<String>,
) -> Result<HashMap<String, i32>, String> {
    if proxies.is_empty() || target_nodes.is_empty() {
        return Ok(HashMap::new());
    }

    // 1. If pool is already running, use its active controller port
    let is_running = {
        let mut child_lock = state.child.lock().unwrap();
        if let Some(ref mut child) = *child_lock {
            matches!(child.try_wait(), Ok(None))
        } else {
            false
        }
    };

    if is_running {
        return Ok(batch_test_latencies(state.controller_port, target_nodes).await);
    }

    // 2. Otherwise, spawn an ephemeral test instance of Mihomo purely for latency testing
    let mihomo_bin = locate_mihomo_binary(app)?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用目录失败: {}", e))?;
    std::fs::create_dir_all(&app_data_dir).map_err(|e| format!("创建数据目录失败: {}", e))?;

    let test_port = find_available_port("127.0.0.1", 19092);

    let mut root = serde_yaml::Mapping::new();
    root.insert(serde_yaml::Value::String("mode".to_string()), serde_yaml::Value::String("direct".to_string()));
    root.insert(serde_yaml::Value::String("log-level".to_string()), serde_yaml::Value::String("silent".to_string()));
    root.insert(serde_yaml::Value::String("ipv6".to_string()), serde_yaml::Value::Bool(false));
    root.insert(
        serde_yaml::Value::String("external-controller".to_string()),
        serde_yaml::Value::String(format!("127.0.0.1:{}", test_port)),
    );
    root.insert(serde_yaml::Value::String("secret".to_string()), serde_yaml::Value::String("".to_string()));

    let mut dns = serde_yaml::Mapping::new();
    dns.insert(serde_yaml::Value::String("enable".to_string()), serde_yaml::Value::Bool(false));
    root.insert(serde_yaml::Value::String("dns".to_string()), serde_yaml::Value::Mapping(dns));

    let test_config_path = app_data_dir.join("mihomo_test_ephemeral.yaml");

    // Auto-clean malformed proxies that fail Mihomo's parser
    let mut valid_proxies = proxies;
    for _ in 0..10 {
        let mut test_root = root.clone();
        test_root.insert(serde_yaml::Value::String("proxies".to_string()), serde_yaml::Value::Sequence(valid_proxies.clone()));
        let test_yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(test_root)).unwrap_or_default();
        std::fs::write(&test_config_path, &test_yaml).map_err(|e| format!("写入测试配置失败: {}", e))?;

        let check = Command::new(&mihomo_bin)
            .arg("-t")
            .arg("-f")
            .arg(&test_config_path)
            .output();

        if let Ok(out) = check {
            if out.status.success() {
                break;
            } else {
                let err_text = String::from_utf8_lossy(&out.stderr);
                if let Some(pos) = err_text.find("proxy ") {
                    let rest = &err_text[pos + 6..];
                    if let Some(colon) = rest.find(':') {
                        if let Ok(idx) = rest[..colon].trim().parse::<usize>() {
                            if idx < valid_proxies.len() {
                                valid_proxies.remove(idx);
                                continue;
                            }
                        }
                    }
                }
                break;
            }
        } else {
            break;
        }
    }

    let mut cmd = Command::new(&mihomo_bin);
    cmd.arg("-d")
        .arg(&app_data_dir)
        .arg("-f")
        .arg(&test_config_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("启动测试内核失败: {}", e))?;

    // Poll until controller is ready (200 OK on /version) or process exits
    let mut ready = false;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap_or_default();
    let version_url = format!("http://127.0.0.1:{}/version", test_port);

    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(Some(status)) = child.try_wait() {
            let mut stderr_msg = String::new();
            if let Some(mut err_reader) = child.stderr.take() {
                use std::io::Read;
                let _ = err_reader.read_to_string(&mut stderr_msg);
            }
            let _ = std::fs::remove_file(&test_config_path);
            return Err(format!("Mihomo 测试内核退出 ({}): {}", status, stderr_msg.trim()));
        }

        if let Ok(resp) = client.get(&version_url).send().await {
            if resp.status().is_success() {
                ready = true;
                break;
            }
        }
    }

    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&test_config_path);
        return Err("Mihomo 测试内核未能就绪，控制端口响应超时".to_string());
    }

    // Run tests
    let results = batch_test_latencies(test_port, target_nodes).await;

    // Clean up test child process
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&test_config_path);

    Ok(results)
}

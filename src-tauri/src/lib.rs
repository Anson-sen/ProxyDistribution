mod core;
mod parser;

use core::{CoreState, PoolStatus, StartPoolRequest};
use parser::ProxyNode;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
async fn fetch_subscription(url: String) -> Result<Vec<ProxyNode>, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("请输入有效的 HTTP 或 HTTPS 订阅链接".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("ClashMeta/1.19.32")
        .build()
        .map_err(|e| format!("创建请求客户端失败: {}", e))?;

    let resp = client
        .get(&url)
        .header("Accept", "*/*")
        .send()
        .await
        .map_err(|e| format!("请求订阅失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("订阅服务器返回错误状态码: {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取订阅响应内容失败: {}", e))?;

    parser::parse_subscription_content(&body)
}

#[tauri::command]
fn parse_text(content: String) -> Result<Vec<ProxyNode>, String> {
    parser::parse_subscription_content(&content)
}

#[tauri::command]
fn read_file_and_parse(file_path: String) -> Result<Vec<ProxyNode>, String> {
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("读取文件失败 ({}): {}", file_path, e))?;
    parser::parse_subscription_content(&content)
}

#[tauri::command]
fn start_pool(
    app: AppHandle,
    state: State<'_, CoreState>,
    req: StartPoolRequest,
) -> Result<PoolStatus, String> {
    core::start_mihomo_instance(&app, &state, req)
}

#[tauri::command]
fn stop_pool(state: State<'_, CoreState>) -> Result<(), String> {
    core::stop_mihomo_instance(&state)
}

#[tauri::command]
fn get_status(state: State<'_, CoreState>) -> Result<PoolStatus, String> {
    Ok(core::get_pool_status(&state))
}

#[tauri::command]
async fn test_single_delay(
    app: AppHandle,
    state: State<'_, CoreState>,
    proxies: Vec<serde_yaml::Value>,
    node_name: String,
) -> Result<i32, String> {
    let res = core::test_nodes_flexible(&app, &state, proxies, vec![node_name.clone()]).await?;
    Ok(res.get(&node_name).copied().unwrap_or(-1))
}

#[tauri::command]
async fn test_all_delays(
    app: AppHandle,
    state: State<'_, CoreState>,
    proxies: Vec<serde_yaml::Value>,
    nodes: Vec<String>,
) -> Result<HashMap<String, i32>, String> {
    core::test_nodes_flexible(&app, &state, proxies, nodes).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let core_state = CoreState {
        child: Mutex::new(None),
        port_mappings: Mutex::new(Vec::new()),
        controller_port: 19090,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(core_state)
        .invoke_handler(tauri::generate_handler![
            fetch_subscription,
            parse_text,
            read_file_and_parse,
            start_pool,
            stop_pool,
            get_status,
            test_single_delay,
            test_all_delays
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Ensure Mihomo child process is cleanly terminated on exit
                let state: State<'_, CoreState> = app_handle.state();
                let _ = core::stop_mihomo_instance(&state);
            }
        });
}

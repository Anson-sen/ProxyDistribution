use base64::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyNode {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub server: String,
    pub port: u16,
    #[serde(default)]
    pub delay: Option<i32>,
    pub raw: serde_yaml::Value,
}

#[derive(Deserialize)]
struct ClashConfig {
    #[serde(default)]
    proxies: Vec<serde_yaml::Value>,
}

pub fn parse_subscription_content(content: &str) -> Result<Vec<ProxyNode>, String> {
    let trimmed = content.trim();

    // 1. Try parsing as Clash YAML first
    if let Ok(config) = serde_yaml::from_str::<ClashConfig>(trimmed) {
        if !config.proxies.is_empty() {
            let mut nodes = Vec::new();
            for p in config.proxies {
                if let Some(node) = yaml_value_to_proxy_node(&p) {
                    nodes.push(node);
                }
            }
            if !nodes.is_empty() {
                return Ok(nodes);
            }
        }
    }

    // 2. Try Base64 decoding (many airport subscriptions are pure base64 text)
    let decoded_text = decode_base64_multiline(trimmed);
    let text_to_parse = if !decoded_text.is_empty() {
        &decoded_text
    } else {
        trimmed
    };

    // 3. Parse line by line for node URIs
    let mut nodes = Vec::new();
    for line in text_to_parse.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(node) = parse_single_uri(line) {
            nodes.push(node);
        }
    }

    if nodes.is_empty() {
        Err("未能解析出任何有效节点，请检查订阅链接或节点文本内容".to_string())
    } else {
        Ok(nodes)
    }
}

fn yaml_value_to_proxy_node(val: &serde_yaml::Value) -> Option<ProxyNode> {
    let mapping = val.as_mapping()?;
    let name = mapping.get(&serde_yaml::Value::String("name".to_string()))?.as_str()?.to_string();
    let node_type = mapping.get(&serde_yaml::Value::String("type".to_string()))?.as_str()?.to_string();
    let server = mapping.get(&serde_yaml::Value::String("server".to_string()))?.as_str()?.to_string();
    let port = mapping.get(&serde_yaml::Value::String("port".to_string()))?.as_u64()? as u16;

    Some(ProxyNode {
        name,
        node_type,
        server,
        port,
        delay: None,
        raw: val.clone(),
    })
}

fn decode_base64_multiline(input: &str) -> String {
    // Strip whitespace and newlines
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if let Ok(bytes) = BASE64_STANDARD.decode(&clean) {
        if let Ok(s) = String::from_utf8(bytes) {
            return s;
        }
    }
    // Try URL-safe base64
    if let Ok(bytes) = BASE64_URL_SAFE_NO_PAD.decode(&clean) {
        if let Ok(s) = String::from_utf8(bytes) {
            return s;
        }
    }
    String::new()
}

pub fn parse_single_uri(uri: &str) -> Option<ProxyNode> {
    if uri.starts_with("ss://") {
        parse_ss(uri)
    } else if uri.starts_with("vmess://") {
        parse_vmess(uri)
    } else if uri.starts_with("vless://") {
        parse_vless(uri)
    } else if uri.starts_with("trojan://") {
        parse_trojan(uri)
    } else if uri.starts_with("hysteria2://") || uri.starts_with("hy2://") {
        parse_hysteria2(uri)
    } else {
        None
    }
}

fn parse_ss(uri: &str) -> Option<ProxyNode> {
    let without_prefix = uri.strip_prefix("ss://")?;
    let (body, fragment) = match without_prefix.split_once('#') {
        Some((b, f)) => (b, urlencoding_decode(f)),
        None => (without_prefix, "SS-Node".to_string()),
    };

    // Format 1: ss://BASE64(cipher:password@server:port)
    // Format 2: ss://BASE64(cipher:password)@server:port
    let (cipher, password, server, port) = if body.contains('@') {
        let (user_info, host_port) = body.split_once('@')?;
        let user_info_decoded = if let Ok(bytes) = BASE64_STANDARD.decode(user_info) {
            String::from_utf8(bytes).ok()?
        } else {
            user_info.to_string()
        };
        let (c, p) = user_info_decoded.split_once(':')?;
        let (s, port_str) = host_port.split_once(':')?;
        (c.to_string(), p.to_string(), s.to_string(), port_str.parse::<u16>().ok()?)
    } else {
        let decoded = if let Ok(bytes) = BASE64_STANDARD.decode(body) {
            String::from_utf8(bytes).ok()?
        } else {
            return None;
        };
        let (user_info, host_port) = decoded.split_once('@')?;
        let (c, p) = user_info.split_once(':')?;
        let (s, port_str) = host_port.split_once(':')?;
        (c.to_string(), p.to_string(), s.to_string(), port_str.parse::<u16>().ok()?)
    };

    let mut map = serde_yaml::Mapping::new();
    map.insert(serde_yaml::Value::String("name".to_string()), serde_yaml::Value::String(fragment.clone()));
    map.insert(serde_yaml::Value::String("type".to_string()), serde_yaml::Value::String("ss".to_string()));
    map.insert(serde_yaml::Value::String("server".to_string()), serde_yaml::Value::String(server.clone()));
    map.insert(serde_yaml::Value::String("port".to_string()), serde_yaml::Value::Number(port.into()));
    map.insert(serde_yaml::Value::String("cipher".to_string()), serde_yaml::Value::String(cipher));
    map.insert(serde_yaml::Value::String("password".to_string()), serde_yaml::Value::String(password));

    let val = serde_yaml::Value::Mapping(map);
    Some(ProxyNode {
        name: fragment,
        node_type: "ss".to_string(),
        server,
        port,
        delay: None,
        raw: val,
    })
}

fn parse_vmess(uri: &str) -> Option<ProxyNode> {
    let body = uri.strip_prefix("vmess://")?;
    let decoded = if let Ok(bytes) = BASE64_STANDARD.decode(body) {
        String::from_utf8(bytes).ok()?
    } else {
        return None;
    };

    let json: serde_json::Value = serde_json::from_str(&decoded).ok()?;
    let name = json.get("ps").and_then(|v| v.as_str()).unwrap_or("VMess-Node").to_string();
    let server = json.get("add").and_then(|v| v.as_str())?.to_string();
    let port = json.get("port").and_then(|v| {
        v.as_u64().or_else(|| v.as_str()?.parse().ok())
    })? as u16;
    let uuid = json.get("id").and_then(|v| v.as_str())?.to_string();
    let aid = json.get("aid").and_then(|v| {
        v.as_u64().or_else(|| v.as_str()?.parse().ok())
    }).unwrap_or(0);
    let net = json.get("net").and_then(|v| v.as_str()).unwrap_or("tcp").to_string();
    let tls = json.get("tls").and_then(|v| v.as_str()).unwrap_or("") == "tls";

    let mut map = serde_yaml::Mapping::new();
    map.insert(serde_yaml::Value::String("name".to_string()), serde_yaml::Value::String(name.clone()));
    map.insert(serde_yaml::Value::String("type".to_string()), serde_yaml::Value::String("vmess".to_string()));
    map.insert(serde_yaml::Value::String("server".to_string()), serde_yaml::Value::String(server.clone()));
    map.insert(serde_yaml::Value::String("port".to_string()), serde_yaml::Value::Number(port.into()));
    map.insert(serde_yaml::Value::String("uuid".to_string()), serde_yaml::Value::String(uuid));
    map.insert(serde_yaml::Value::String("alterId".to_string()), serde_yaml::Value::Number(aid.into()));
    map.insert(serde_yaml::Value::String("cipher".to_string()), serde_yaml::Value::String("auto".to_string()));
    map.insert(serde_yaml::Value::String("network".to_string()), serde_yaml::Value::String(net));
    map.insert(serde_yaml::Value::String("tls".to_string()), serde_yaml::Value::Bool(tls));

    if let Some(host) = json.get("host").and_then(|v| v.as_str()) {
        if !host.is_empty() {
            map.insert(serde_yaml::Value::String("servername".to_string()), serde_yaml::Value::String(host.to_string()));
        }
    }
    if let Some(path) = json.get("path").and_then(|v| v.as_str()) {
        if !path.is_empty() {
            let mut ws_opts = serde_yaml::Mapping::new();
            ws_opts.insert(serde_yaml::Value::String("path".to_string()), serde_yaml::Value::String(path.to_string()));
            map.insert(serde_yaml::Value::String("ws-opts".to_string()), serde_yaml::Value::Mapping(ws_opts));
        }
    }

    let val = serde_yaml::Value::Mapping(map);
    Some(ProxyNode {
        name,
        node_type: "vmess".to_string(),
        server,
        port,
        delay: None,
        raw: val,
    })
}

fn parse_vless(uri: &str) -> Option<ProxyNode> {
    let parsed_url = Url::parse(uri).ok()?;
    let uuid = parsed_url.username().to_string();
    let server = parsed_url.host_str()?.to_string();
    let port = parsed_url.port()?;
    let name = parsed_url.fragment().map(urlencoding_decode).unwrap_or_else(|| "VLESS-Node".to_string());

    let query_pairs: HashMap<String, String> = parsed_url.query_pairs().into_owned().collect();

    let mut map = serde_yaml::Mapping::new();
    map.insert(serde_yaml::Value::String("name".to_string()), serde_yaml::Value::String(name.clone()));
    map.insert(serde_yaml::Value::String("type".to_string()), serde_yaml::Value::String("vless".to_string()));
    map.insert(serde_yaml::Value::String("server".to_string()), serde_yaml::Value::String(server.clone()));
    map.insert(serde_yaml::Value::String("port".to_string()), serde_yaml::Value::Number(port.into()));
    map.insert(serde_yaml::Value::String("uuid".to_string()), serde_yaml::Value::String(uuid));

    if let Some(flow) = query_pairs.get("flow") {
        if !flow.is_empty() {
            map.insert(serde_yaml::Value::String("flow".to_string()), serde_yaml::Value::String(flow.clone()));
        }
    }
    let net = query_pairs.get("type").map(|s| s.as_str()).unwrap_or("tcp");
    map.insert(serde_yaml::Value::String("network".to_string()), serde_yaml::Value::String(net.to_string()));

    let security = query_pairs.get("security").map(|s| s.as_str()).unwrap_or("");
    if security == "reality" {
        map.insert(serde_yaml::Value::String("tls".to_string()), serde_yaml::Value::Bool(true));
        let mut reality_opts = serde_yaml::Mapping::new();
        if let Some(pbk) = query_pairs.get("pbk") {
            reality_opts.insert(serde_yaml::Value::String("public-key".to_string()), serde_yaml::Value::String(pbk.clone()));
        }
        if let Some(sid) = query_pairs.get("sid") {
            reality_opts.insert(serde_yaml::Value::String("short-id".to_string()), serde_yaml::Value::String(sid.clone()));
        }
        map.insert(serde_yaml::Value::String("reality-opts".to_string()), serde_yaml::Value::Mapping(reality_opts));
    } else if security == "tls" {
        map.insert(serde_yaml::Value::String("tls".to_string()), serde_yaml::Value::Bool(true));
    }

    if let Some(sni) = query_pairs.get("sni") {
        map.insert(serde_yaml::Value::String("servername".to_string()), serde_yaml::Value::String(sni.clone()));
    }
    if let Some(fp) = query_pairs.get("fp") {
        map.insert(serde_yaml::Value::String("client-fingerprint".to_string()), serde_yaml::Value::String(fp.clone()));
    }

    let val = serde_yaml::Value::Mapping(map);
    Some(ProxyNode {
        name,
        node_type: "vless".to_string(),
        server,
        port,
        delay: None,
        raw: val,
    })
}

fn parse_trojan(uri: &str) -> Option<ProxyNode> {
    let parsed_url = Url::parse(uri).ok()?;
    let password = parsed_url.username().to_string();
    let server = parsed_url.host_str()?.to_string();
    let port = parsed_url.port()?;
    let name = parsed_url.fragment().map(urlencoding_decode).unwrap_or_else(|| "Trojan-Node".to_string());

    let query_pairs: HashMap<String, String> = parsed_url.query_pairs().into_owned().collect();

    let mut map = serde_yaml::Mapping::new();
    map.insert(serde_yaml::Value::String("name".to_string()), serde_yaml::Value::String(name.clone()));
    map.insert(serde_yaml::Value::String("type".to_string()), serde_yaml::Value::String("trojan".to_string()));
    map.insert(serde_yaml::Value::String("server".to_string()), serde_yaml::Value::String(server.clone()));
    map.insert(serde_yaml::Value::String("port".to_string()), serde_yaml::Value::Number(port.into()));
    map.insert(serde_yaml::Value::String("password".to_string()), serde_yaml::Value::String(password));

    if let Some(sni) = query_pairs.get("sni") {
        map.insert(serde_yaml::Value::String("sni".to_string()), serde_yaml::Value::String(sni.clone()));
    }

    let val = serde_yaml::Value::Mapping(map);
    Some(ProxyNode {
        name,
        node_type: "trojan".to_string(),
        server,
        port,
        delay: None,
        raw: val,
    })
}

fn parse_hysteria2(uri: &str) -> Option<ProxyNode> {
    let raw = uri.replace("hy2://", "hysteria2://");
    let parsed_url = Url::parse(&raw).ok()?;
    let password = parsed_url.username().to_string();
    let server = parsed_url.host_str()?.to_string();
    let port = parsed_url.port()?;
    let name = parsed_url.fragment().map(urlencoding_decode).unwrap_or_else(|| "Hysteria2-Node".to_string());

    let query_pairs: HashMap<String, String> = parsed_url.query_pairs().into_owned().collect();

    let mut map = serde_yaml::Mapping::new();
    map.insert(serde_yaml::Value::String("name".to_string()), serde_yaml::Value::String(name.clone()));
    map.insert(serde_yaml::Value::String("type".to_string()), serde_yaml::Value::String("hysteria2".to_string()));
    map.insert(serde_yaml::Value::String("server".to_string()), serde_yaml::Value::String(server.clone()));
    map.insert(serde_yaml::Value::String("port".to_string()), serde_yaml::Value::Number(port.into()));
    map.insert(serde_yaml::Value::String("password".to_string()), serde_yaml::Value::String(password));

    if let Some(sni) = query_pairs.get("sni") {
        map.insert(serde_yaml::Value::String("sni".to_string()), serde_yaml::Value::String(sni.clone()));
    }
    if let Some(insecure) = query_pairs.get("insecure") {
        let skip = insecure == "1" || insecure == "true";
        map.insert(serde_yaml::Value::String("skip-cert-verify".to_string()), serde_yaml::Value::Bool(skip));
    }

    let val = serde_yaml::Value::Mapping(map);
    Some(ProxyNode {
        name,
        node_type: "hysteria2".to_string(),
        server,
        port,
        delay: None,
        raw: val,
    })
}

fn urlencoding_decode(s: &str) -> String {
    url::form_urlencoded::parse(s.as_bytes())
        .map(|(k, _)| k.to_string())
        .collect::<Vec<_>>()
        .join("")
}

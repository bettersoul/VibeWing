use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
pub struct ChatRequest {
    pub provider_id: String,
    pub model: String,
    pub messages: Vec<ChatMessage>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ChatResponse {
    pub content: String,
    pub elapsed_ms: u128,
}

/// 把 reqwest 的网络层错误翻译成具体的、可操作的提示。
fn describe_reqwest_err(e: reqwest::Error) -> String {
    let detail = e.to_string();
    let lower = detail.to_lowercase();
    if e.is_timeout() {
        return format!(
            "请求超时：服务器在限定时间内未响应。请检查 base_url 是否填错、网络是否稳定。\n详细：{}",
            detail
        );
    }
    if e.is_connect() {
        if lower.contains("certificate") || lower.contains("ssl") || lower.contains("tls") {
            return format!(
                "TLS/证书校验失败，无法建立安全连接（常见于自建服务的自签证书或代理拦截）。\n详细：{}",
                detail
            );
        }
        return format!(
            "无法连接到服务器（连接被拒绝或主机不可达）。请检查 base_url 是否正确、目标服务是否正在运行。\n详细：{}",
            detail
        );
    }
    if e.is_redirect() {
        return format!("请求被重定向且出错，请检查 base_url 是否指向了会跳转的登录页。\n详细：{}", detail);
    }
    format!("请求发送失败：{}", detail)
}

/// 把 HTTP 状态码翻译成具体的、可操作的提示。
fn describe_status(status: u16, snippet: &str) -> String {
    let reason = match status {
        400 => "请求参数有误（如 model 名称不存在或请求体格式错误）",
        401 => "API Key 无效或缺失，请检查设置里的 API Key 是否正确",
        403 => "无权限访问：可能是 API Key 没有该模型的权限、额度已用完或地区受限",
        404 => "接口地址不存在（404）。请检查 base_url 是否拼错，或是否漏了版本路径（OpenAI 兼容服务需以 /v1 结尾）",
        405 => "请求方法不被允许（405），请确认接口路径正确",
        408 => "请求超时（408），服务器未在限定时间内处理完成",
        413 => "请求体过大（413），请减少输入内容",
        429 => "触发速率限制（429），请求过于频繁，请稍后重试",
        500 => "服务端内部错误（500）",
        502 => "网关错误（502），上游服务可能未正常启动",
        503 => "服务不可用（503），上游服务可能正在重启或过载",
        504 => "网关超时（504），上游服务响应过慢",
        _ => "请求未被服务器接受",
    };
    format!("HTTP {}：{}\n服务返回内容：{}", status, reason, snippet)
}

pub async fn complete(
    request: ChatRequest,
    base_url: &str,
    api_key: &str,
) -> Result<ChatResponse, String> {
    let started = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{}", e))?;
    let response = client
        .post(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .json(&serde_json::json!({"model":request.model,"messages":request.messages,"temperature":0.2,"max_tokens":2048,"stream":false}))
        .send()
        .await
        .map_err(describe_reqwest_err)?;
    let status = response.status();
    let raw = response
        .text()
        .await
        .map_err(|e| format!("读取响应内容失败：{}", e))?;
    if !status.is_success() {
        let snippet: String = raw.chars().take(500).collect();
        return Err(describe_status(status.as_u16(), &snippet));
    }
    let body: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        let lower = raw.to_lowercase();
        let is_html = lower.contains("<!doctype")
            || lower.contains("<html")
            || lower.trim_start().starts_with('<');
        if is_html {
            return format!(
                "服务端返回的是网页而非 JSON 数据，通常是因为 base_url 填写错误（如未包含 /v1 路径或地址拼错），导致请求落到了登录页/错误页。\n原始内容：{}",
                raw.chars().take(500).collect::<String>()
            );
        }
        format!(
            "响应不是合法的 JSON（{}）。原始内容：{}",
            e,
            raw.chars().take(500).collect::<String>()
        )
    })?;
    let content = body
        .get("choices")
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("message"))
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if content.is_empty() {
        return Err("模型返回了空内容，请检查 model 名称是否正确或接口是否兼容 OpenAI 格式".into());
    }
    Ok(ChatResponse {
        content,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

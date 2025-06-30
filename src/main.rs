use axum::{
    extract::{RawQuery, State},
    http::{self, HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Router,
};
use futures_util::stream::StreamExt;
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::process::{Child, Command};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

const REDIS_HOST: &str = "REDIS_HOST";
const REDIS_URL: &str = "REDIS_URL";
const DB_HOST: &str = "DB_HOST";
const DB_URL: &str = "DB_URL";
const CHANNEL_GATE2VIN: &str = "gate2vin";
const CHANNEL_VIN2WORKER: &str = "vin2worker";
pub(crate) const ACTION_NEW_BLOCK_HEIGHT: &str = "block_height";
pub(crate) const ACTION_UPLOAD_WASM: &str = "upload_wasm";
pub(crate) const ACTION_UPGRADE_WASM: &str = "upgrade_wasm";

#[derive(Debug, Serialize, Deserialize)]
struct Info {
    proto: String,
    version: String,
    digest: String,
    afterblocks: usize,
    timestamp: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InputOutputObject {
    proto: String,
    model: String,
    action: String,
    data: Vec<u8>,
    ext: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
struct BlockInfo {
    block_height: u64,
    block_hash: String,
}

#[derive(Clone)]
struct AppState {
    redis_client: Client,
    // spin_tasks: Arc<Mutex<HashMap<String, Child>>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let redis_url = std::env::var(REDIS_URL)?;
    let redis_client = Client::open(redis_url)?;

    // Initialize shared state
    let state = AppState {
        redis_client,
        // spin_tasks: Arc::new(Mutex::new(HashMap::new())),
    };

    // Start Redis pub/sub listener in a separate task
    let redis_client_clone = state.redis_client.clone();
    tokio::spawn(async move {
        if let Err(e) = run_redis_listener(redis_client_clone).await {
            log::error!("Redis listener error: {}", e);
        }
    });

    // Set up HTTP server
    let app = Router::new()
        .route("/{*path}", get(handle_get))
        .route("/{*path}", post(handle_post))
        .route("/{*path}", put(handle_put))
        .route("/{*path}", delete(handle_delete))
        .route(
            "/{*path}",
            axum::routing::on(axum::routing::MethodFilter::OPTIONS, handle_options),
        )
        .with_state(state);

    let addr = "0.0.0.0:3000";
    log::info!("Starting server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_redis_listener(client: Client) -> anyhow::Result<()> {
    // let con = client.get_multiplexed_async_connection().await?;
    let (mut sink, mut stream) = client.get_async_pubsub().await?.split();
    sink.subscribe(CHANNEL_VIN2WORKER).await?;
    log::info!("Listening for messages on '{}'", CHANNEL_VIN2WORKER);

    let spin_tasks = Arc::new(Mutex::new(HashMap::new()));

    while let Some(msg) = stream.next().await {
        let payload: String = msg.get_payload()?;
        match serde_json::from_str::<InputOutputObject>(&payload) {
            Ok(message) => {
                // log::info!(
                //     "Received message: {} {} {} {} {}",
                //     message.action,
                //     message.proto,
                //     message.model,
                //     message.data.len(),
                //     message.ext.len()
                // );
                if let Err(e) = process_message(message, &spin_tasks).await {
                    log::error!("Error processing message: {}", e);
                }
            }
            Err(e) => {
                log::error!("Failed to deserialize message: {}", e);
            }
        }
    }
    Ok(())
}

async fn process_message(
    msg: InputOutputObject,
    spin_tasks: &Arc<Mutex<HashMap<String, Child>>>,
) -> anyhow::Result<()> {
    match msg.action.as_str() {
        ACTION_NEW_BLOCK_HEIGHT => {
            // log::info!("msgdata: {:?}", msg);
            // Deserialize back to BlockInfo
            let _block_info: BlockInfo = serde_json::from_slice(&msg.data)?;

            // let body: [u8; 8] = msg
            //     .data
            //     .try_into()
            //     .map_err(|_| anyhow::anyhow!("Invalid data length"))?;
            // let block_height = u64::from_be_bytes(body);
            // log::info!(
            //     "Block height, hash: {} {}",
            //     block_info.block_height,
            //     block_info.block_hash
            // );
        }
        ACTION_UPLOAD_WASM => {
            let wasm_hash = hex::encode(&msg.data);
            let wasm_binary = msg.ext;
            let path = format!("wasm_files/{}.wasm", wasm_hash);
            let mut output_file = fs::File::create(&path)?;
            output_file.write_all(&wasm_binary)?;
            log::info!("WASM file {} saved", path);
        }
        ACTION_UPGRADE_WASM => {
            let template = fs::read_to_string("spin_tmpl.toml")?;
            let wasm_hash = hex::encode(&msg.data);
            let proto = &msg.proto;

            let replacements = [
                ("$proto_id", proto.as_str()),
                ("$wasm_hash", wasm_hash.as_str()),
            ];
            let mut result = template;
            for (pattern, replacement) in replacements.iter() {
                result = result.replace(pattern, replacement);
            }

            let path = format!("tmp_configs/{}-{}.toml", proto, wasm_hash);
            let mut output_file = fs::File::create(&path)?;
            output_file.write_all(result.as_bytes())?;
            log::info!("Generated spin config: {}", path);

            let redis_host = std::env::var(REDIS_HOST)?;
            let db_host = std::env::var(DB_HOST)?;
            let redis_url = std::env::var(REDIS_URL)?;
            let db_url = std::env::var(DB_URL)?.replace("#proto", proto);
            let redis_env = format!("REDIS_URL={}", redis_url);
            let db_env = format!("DB_URL={}", db_url);
            log::info!("redis and db: {} {}", redis_env, db_env);
            let outbound_host1 = std::env::var("OUTBOUND_HOST1")?;
            let outbound_host2 = std::env::var("OUTBOUND_HOST2")?;
            let outbound_host3 = std::env::var("OUTBOUND_HOST3")?;

            let mut env_vars = HashMap::new();
            for (key, value) in std::env::vars() {
                if key.starts_with("SPIN_ENV_") {
                    println!("collecting env: {}: {}", key, value);
                    env_vars.insert(key, value);
                }
            }
            env_vars.insert("SPIN_VARIABLE_REDIS_HOST".to_string(), redis_host.clone());
            env_vars.insert("SPIN_VARIABLE_DB_HOST".to_string(), db_host.clone());
            env_vars.insert("SPIN_VARIABLE_PROTO_ID".to_string(), proto.clone());
            env_vars.insert("SPIN_VARIABLE_WASM_HASH".to_string(), wasm_hash.clone());
            env_vars.insert(
                "SPIN_VARIABLE_OUTBOUND_HOST1".to_string(),
                outbound_host1.clone(),
            );
            env_vars.insert(
                "SPIN_VARIABLE_OUTBOUND_HOST2".to_string(),
                outbound_host2.clone(),
            );
            env_vars.insert(
                "SPIN_VARIABLE_OUTBOUND_HOST3".to_string(),
                outbound_host3.clone(),
            );

            let mut spin_tasks = spin_tasks.lock().await;
            if let Some(child) = spin_tasks.get_mut(proto) {
                log::info!("Killing the old version of {}", proto);
                send_ctrl_c(child)?;
            }

            let child = run_command_with_env(
                "spin",
                &["up", "-f", &path, "-e", &redis_env, "-e", &db_env],
                env_vars,
            );
            spin_tasks.insert(proto.clone(), child);
            log::info!("Protocol {} has upgraded to version: {}", proto, wasm_hash);
        }
        _ => {
            log::error!("Unknown action type: {}", msg.action);
        }
    }
    Ok(())
}

fn run_command_with_env(command: &str, args: &[&str], env_vars: HashMap<String, String>) -> Child {
    Command::new(command)
        .args(args)
        .envs(env_vars)
        .spawn()
        .expect("Failed to execute child")
}

fn send_ctrl_c(child: &mut Child) -> anyhow::Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(child.id() as i32), Signal::SIGINT)?;
    Ok(())
}

fn parse_proto_name(path: &str) -> String {
    path.trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

async fn handle_get(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    RawQuery(query_string): RawQuery,
    headers: HeaderMap,
) -> impl IntoResponse {
    let query_string = query_string.unwrap_or_default(); // Use empty string if None
    log::info!("in handle_get: query_string: {}", query_string);

    let params_bytes = axum::body::Bytes::from(query_string);
    // // Optionally convert to bytes for your handler
    // let params_bytes: axum::body::Bytes =
    //     serde_json::to_vec(&query_string).unwrap_or_default().into();
    handle_request(Method::GET, &path, headers, params_bytes, state).await
}

async fn handle_post(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    handle_request(Method::POST, &path, headers, body, state).await
}

async fn handle_put(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    handle_request(Method::PUT, &path, headers, body, state).await
}

async fn handle_delete(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    handle_request(Method::DELETE, &path, headers, body, state).await
}

async fn handle_options() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert("ef-http-gate-version", "1.0".parse().unwrap());
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
    headers.insert(
        "Access-Control-Allow-Methods",
        "GET, POST, PUT, DELETE, OPTIONS".parse().unwrap(),
    );
    headers.insert(
        "Access-Control-Allow-Headers",
        "X-PINGOTHER, Content-Type".parse().unwrap(),
    );
    (StatusCode::OK, headers, "No data")
}

async fn handle_request<T>(
    method: Method,
    path: &str,
    headers: HeaderMap,
    reqdata: T,
    state: AppState,
) -> impl IntoResponse
where
    T: Into<axum::body::Bytes>,
{
    let proto_name = parse_proto_name(path);
    if proto_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "proto_name is empty".to_string()).into_response();
    }
    log::info!("in handle_request: method: {}", method);
    log::info!("in handle_request: path: {}", path);

    let reqdata = reqdata.into();
    let reqdata_str = if !reqdata.is_empty() {
        Some(String::from_utf8_lossy(&reqdata))
    } else {
        None
    };
    log::info!("in handle_request: reqdata: {:?}", reqdata_str);

    let method_str = match method {
        Method::GET => "get",
        Method::POST => "post",
        Method::PUT => "put",
        Method::DELETE => "delete",
        _ => {
            return (
                StatusCode::METHOD_NOT_ALLOWED,
                "Unsupported HTTP method".to_string(),
            )
                .into_response()
        }
    };

    let headers_map: HashMap<String, String> = headers
        .into_iter()
        .filter_map(|(name, value)| {
            name.map(|n| {
                (
                    n.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
        })
        .collect();

    let reqid = Uuid::new_v4().simple().to_string();
    let payload = json!({
        "reqid": reqid,
        "reqdata": reqdata_str,
        "reqheaders": headers_map,
    });

    let json_to_send = json!({
        "proto": proto_name,
        "model": path,
        "action": method_str,
        "data": payload.to_string().as_bytes().to_vec(),
        "ext": Vec::<u8>::new(),
    });

    let mut con = match state.redis_client.get_multiplexed_async_connection().await {
        Ok(con) => con,
        Err(e) => {
            log::error!("Redis connection error: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Redis connection failed".to_string(),
            )
                .into_response();
        }
    };

    let channel = match method {
        Method::GET => format!("{}:{}", CHANNEL_VIN2WORKER, proto_name),
        Method::POST | Method::PUT | Method::DELETE => CHANNEL_GATE2VIN.to_string(),
        _ => unreachable!(),
    };

    if let Err(e) = con
        .publish::<_, _, ()>(&channel, serde_json::to_vec(&json_to_send).unwrap())
        .await
    {
        log::error!("Redis publish error: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to publish to Redis".to_string(),
        )
            .into_response();
    }

    let mut loop_count = 0;
    loop {
        let status_code: Option<Vec<u8>> = con
            .get(format!("cache:status:{}", reqid))
            .await
            .unwrap_or(None);
        if let Some(status_code) = status_code {
            let res_headers: Option<Vec<u8>> = con
                .get(format!("cache:headers:{}", reqid))
                .await
                .unwrap_or(None);
            let res_body: Option<Vec<u8>> =
                con.get(format!("cache:{}", reqid)).await.unwrap_or(None);

            // clear cache
            let _: () = con
                .del(&[
                    format!("cache:status:{}", reqid),
                    format!("cache:headers:{}", reqid),
                    format!("cache:{}", reqid),
                ])
                .await
                .unwrap_or(());

            let status_code = String::from_utf8(status_code).unwrap_or("500".to_string());
            let status_code = status_code.parse::<u16>().unwrap();

            // processing response headers
            let res_headers = res_headers.unwrap_or_default();
            let headers: HashMap<String, String> =
                serde_json::from_slice(&res_headers).unwrap_or_default();
            let mut headers: HeaderMap = (&headers).try_into().expect("headers not valid.");
            headers.insert("ef-http-gate-version", "1.0".parse().unwrap());
            headers.insert(
                http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                "*".parse().unwrap(),
            );

            let res_body = res_body.unwrap_or_default();

            return (
                StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                headers,
                res_body,
            )
                .into_response();
        }

        if loop_count >= 1000 {
            let mut headers = HeaderMap::new();
            headers.insert(
                http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                "*".parse().unwrap(),
            );
            return (
                StatusCode::REQUEST_TIMEOUT,
                headers,
                "Request Timeout".to_string(),
            )
                .into_response();
        }

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        loop_count += 1;
    }
}

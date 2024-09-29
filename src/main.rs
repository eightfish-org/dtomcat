use redis::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::process::Child;
use std::process::Command;
// use std::thread;

const CHANNEL_VIN2WORKER: &str = "vin2worker";

// dtomcat action
pub(crate) const ACTION_NEW_BLOCK_HEIGHT: &str = "block_height";
pub(crate) const ACTION_UPLOAD_WASM: &str = "upload_wasm";
pub(crate) const ACTION_UPGRADE_WASM: &str = "upgrade_wasm";

// #[derive(Debug, Serialize, Deserialize)]
// enum EventType {
//     UploadWasmFile,
//     DoUpgrade,
// }

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

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let redis_host = std::env::var("REDIS_HOST")?;
    let client = Client::open(format!("redis://{redis_host}"))?;
    let mut con = client.get_connection()?;
    let mut pubsub = con.as_pubsub();

    // Subscribe to the channel
    pubsub.subscribe(CHANNEL_VIN2WORKER)?;

    println!("Listening for messages on 'vin2worker'...");
    let mut spin_tasks: HashMap<String, std::process::Child> = HashMap::new();

    loop {
        let msg = pubsub.get_message()?;
        let payload: String = msg.get_payload()?;

        match serde_json::from_str::<InputOutputObject>(&payload) {
            Ok(message) => {
                log::info!(
                    "Received message: {} {} {} {} {}",
                    message.action,
                    message.proto,
                    message.model,
                    message.data.len(),
                    message.ext.len()
                );
                process_message(message, &mut spin_tasks)?;
            }
            Err(e) => {
                eprintln!("Failed to deserialize message: {}", e);
                continue;
            }
        }
    }
}

fn process_message(
    msg: InputOutputObject,
    spin_tasks: &mut HashMap<String, std::process::Child>,
) -> Result<(), Box<dyn Error>> {
    // println!("Processing message: {:?}", msg);

    match &msg.action[..] {
        ACTION_NEW_BLOCK_HEIGHT => {
            let body: [u8; 8] = msg.data.try_into().unwrap_or([0; 8]);
            // convert to u64
            let block_height = u64::from_be_bytes(body);

            // do something
            log::info!("Block height: {block_height}");
        }
        ACTION_UPLOAD_WASM => {
            let wasm_hash = hex::encode(msg.data);
            let wasm_binary = msg.ext;
            // check the digest of body
            // if wasm_hash != sha256_check(wasm_binary) {}

            // let proto = msg.proto;
            // let version = msg.model;
            // generate a new wasm binary file
            let path = format!("wasm_files/{wasm_hash}.wasm");
            // Write the result to a new file
            let mut output_file = fs::File::create(&path)?;
            output_file.write_all(&wasm_binary)?;

            log::info!("on upload wasm, wasm file {path} saved.");
        }
        ACTION_UPGRADE_WASM => {
            // TODO: check whether the existence of the corresponding wasm file
            // if doesn't exist, return early

            // Read the template file
            let template = fs::read_to_string("spin_tmpl.toml")?;

            // let info: Info = serde_json::from_str(&msg.model)?;
            let wasm_hash = hex::encode(msg.data);
            let proto = msg.proto;
            // Define the replacements
            let replacements = [("$proto_id", &proto), ("$wasm_hash", &wasm_hash)];

            // Perform the replacements
            let mut result = template;
            for (pattern, replacement) in replacements.iter() {
                result = result.replace(pattern, &replacement);
            }

            // generate a new intermediate spin config file
            let path = format!("tmp_configs/{}-{}.toml", proto, wasm_hash);
            // Write the result to a new file
            let mut output_file = fs::File::create(&path)?;
            output_file.write_all(result.as_bytes())?;

            log::info!("Replacement complete. Check output file.");

            // Important:  when start this dtomcat process, we must specify these two envs
            let redis_host = std::env::var("REDIS_HOST")?;
            let db_host = std::env::var("DB_HOST")?;

            // spawn new spin instance
            let mut env_vars = HashMap::new();
            env_vars.insert("SPIN_VARIABLE_REDIS_HOST".to_string(), redis_host.clone());
            env_vars.insert("SPIN_VARIABLE_POSTGRES_HOST".to_string(), db_host.clone());
            env_vars.insert("SPIN_VARIABLE_PROTO_ID".to_string(), proto.clone());
            env_vars.insert("SPIN_VARIABLE_WASM_HASH".to_string(), wasm_hash.clone());

            // let redis_env = "REDIS_URL_ENV='redis://localhost:6379'";
            let redis_env = format!("REDIS_URL_ENV=redis://{redis_host}");
            let db_env = format!(
                "DB_URL_ENV=postgresql://postgres:postgres@{}/{}?sslmode=disable",
                db_host, proto
            );

            // if already has an old version, kill first
            if let Some(child) = spin_tasks.get_mut(&proto) {
                log::info!("now try to kill old version of {proto}.");

                // child.kill().expect("spin task: {proto} couldn't be killed");
                send_ctrl_c(child)?;
            }

            // and then create new task process
            let child = run_command_with_env(
                "spin",
                &["up", "-f", &path, "-e", &redis_env, "-e", &db_env],
                env_vars,
            );
            spin_tasks.insert(proto.clone(), child);

            log::info!(
                "on proto upgrade, the protocol {proto} has been upgraded to version: {wasm_hash}."
            );
        }
        _ => {
            log::error!("error action type in this msg from redis.");
        }
    }

    Ok(())
}

fn run_command_with_env(
    command: &str,
    args: &[&str],
    env_vars: HashMap<String, String>,
) -> std::process::Child {
    let command = command.to_string();
    let args = args.iter().map(|&s| s.to_string()).collect::<Vec<String>>();

    Command::new(command)
        .args(&args)
        .envs(env_vars) // Set environment variables
        .spawn()
        .expect("failed to execute child")

    // let ecode = child.wait().expect("failed to wait on child");
    // assert!(ecode.success());
}

fn send_ctrl_c(child: &mut Child) -> Result<(), Box<dyn std::error::Error>> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    kill(Pid::from_raw(child.id() as i32), Signal::SIGINT)?;
    Ok(())
}

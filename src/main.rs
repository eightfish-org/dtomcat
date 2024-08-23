use redis::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::process::Command;
use std::thread;

const CHANNEL_VIN2WORKER: &str = "vin2worker";

#[derive(Debug, Serialize, Deserialize)]
enum EventType {
    UploadWasmFile,
    DoUpgrade,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    id: u64,
    eventtype: EventType,
    proto_id: String,
    proto_version: String,
    digest: String,
    body: Vec<u8>,
    timestamp: i64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let client = Client::open("redis://127.0.0.1/")?;
    let mut con = client.get_connection()?;
    let mut pubsub = con.as_pubsub();

    // Subscribe to the channel
    pubsub.subscribe(CHANNEL_VIN2WORKER)?;

    println!("Listening for messages on 'vin2worker'...");

    loop {
        let msg = pubsub.get_message()?;
        let payload: String = msg.get_payload()?;

        match serde_json::from_str::<Message>(&payload) {
            Ok(message) => {
                println!("Received message: {:?}", message);
                process_message(message)?;
            }
            Err(e) => {
                eprintln!("Failed to deserialize message: {}", e);
                continue;
            }
        }
    }
}

fn process_message(msg: Message) -> Result<(), Box<dyn Error>> {
    println!("Processing message:");
    println!("ID: {}", msg.id);
    println!("Timestamp: {}", msg.timestamp);

    match msg.eventtype {
        EventType::UploadWasmFile => {
            let body = &msg.body;
            // check the digest of body
            // if msg.digest != sha256_check(body) {}

            // generate a new wasm binary file
            let path = format!(
                "proto_wasm_files/{}-v{}.wasm",
                msg.proto_id, msg.proto_version
            );
            // Write the result to a new file
            let mut output_file = fs::File::create(path)?;
            output_file.write_all(body)?;
        }
        EventType::DoUpgrade => {
            // TODO: check whether the existence of the corresponding wasm file
            // if doesn't exist, return early

            // Read the template file
            let template = fs::read_to_string("spin_tmpl.txt")?;

            // this id will be received from the redis
            let proto_id = &msg.proto_id;
            let proto_version = &msg.proto_version;
            // Define the replacements
            let replacements = [("$proto_id", proto_id), ("$proto_version", proto_version)];

            // Perform the replacements
            let mut result = template;
            for (pattern, replacement) in replacements.iter() {
                result = result.replace(pattern, &replacement);
            }

            // generate a new intermediate spin config file
            let path = format!(
                "proto_config_files/spin-{}-v{}.toml",
                proto_id, proto_version
            );
            // Write the result to a new file
            let mut output_file = fs::File::create(&path)?;
            output_file.write_all(result.as_bytes())?;

            println!("Replacement complete. Check output file.");

            let redis_host = std::env::var("REDIS_HOST")?;
            let db_host = std::env::var("DB_HOST")?;

            // spawn new spin instance
            let mut env_vars = HashMap::new();
            env_vars.insert("SPIN_VARIABLE_REDIS_HOST".to_string(), redis_host.clone());
            env_vars.insert("SPIN_VARIABLE_PROTO_ID".to_string(), msg.proto_id.clone());

            let redis_env = "REDIS_URL_ENV='redis://localhost:6379'";
            let db_env = format!(
                "DB_URL_ENV='host={} user=postgres password=postgres dbname={} sslmode=disable'",
                db_host, msg.proto_id
            );

            let _proto_handle = run_command_with_env(
                "spin",
                &["up", "-f", &path, "-e", &redis_env, "-e", &db_env],
                env_vars,
            );
            // Don't join.
            // match proto_handle.join().expect("Thread panicked") {
            //     Ok(output) => println!(
            //         "ls command output:\n{}",
            //         String::from_utf8_lossy(&output.stdout)
            //     ),
            //     Err(e) => eprintln!("ls command error: {}", e),
            // }
        }
    }

    Ok(())
}

#[allow(dead_code)]
fn run_command(
    command: &str,
    args: &[&str],
) -> thread::JoinHandle<std::io::Result<std::process::Output>> {
    let command = command.to_string();
    let args = args.iter().map(|&s| s.to_string()).collect::<Vec<String>>();

    thread::spawn(move || Command::new(command).args(&args).output())
}

fn run_command_with_env(
    command: &str,
    args: &[&str],
    env_vars: HashMap<String, String>,
) -> thread::JoinHandle<std::io::Result<std::process::Output>> {
    let command = command.to_string();
    let args = args.iter().map(|&s| s.to_string()).collect::<Vec<String>>();

    thread::spawn(move || {
        Command::new(command)
            .args(&args)
            .envs(env_vars) // Set environment variables
            .output()
    })
}

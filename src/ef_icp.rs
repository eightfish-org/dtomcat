use ic_agent::{Agent, Identity};
use candid::{Encode, Decode, Principal};
use chrono::Utc;
use std::path::Path;
use std::fs;
use ic_agent::identity::Secp256k1Identity;
use tokio::runtime::Runtime;

pub struct ICHeartbeat {
    runtime: Runtime,
    agent: Agent,
    canister_id: Principal,
}

impl ICHeartbeat {
    pub fn new(canister_id: &str, url: &str, identity_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let canister_id = Principal::from_text(canister_id)?;
        let identity = Self::load_identity(identity_path)?;
        let agent = Agent::builder()
            .with_url(url)
            .with_identity(identity)
            .build()?;

        runtime.block_on(async {
            agent.fetch_root_key().await
        })?;

        Ok(Self { runtime, agent, canister_id })
    }

    fn load_identity(identity_path: &str) -> Result<impl Identity, Box<dyn std::error::Error>> {
        let pem_file = Path::new(identity_path);
        
        if !pem_file.exists() {
            return Err(format!("PEM file not found at path: {}", identity_path).into());
        }

        let pem_contents = fs::read(pem_file)?;

        let identity = Secp256k1Identity::from_pem(pem_contents.as_slice())
            .map_err(|e| format!("Failed to create identity from PEM: {}", e))?;

        Ok(identity)
    }

    pub fn get_all_heartbeats(&self) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        self.runtime.block_on(async {
            let arg = Encode!(&())?;
            let response = self.agent.query(&self.canister_id, "get_all_heartbeats")
                .with_arg(arg)
                .call()
                .await?;

            let result: Vec<(String, String)> = Decode!(&response, Vec<(String, String)>)?;
            Ok(result)
        })
    }

    pub fn get_last_heartbeat(&self, subject: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        self.runtime.block_on(async {
            let arg = Encode!(&subject)?;
            let response = self.agent.query(&self.canister_id, "get_last_heartbeat")
                .with_arg(arg)
                .call()
                .await?;

            let result: Option<String> = Decode!(&response, Option<String>)?;
            Ok(result)
        })
    }

    pub fn register_protocol(&self, protocol_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.runtime.block_on(async {
            let arg = Encode!(&protocol_name)?;
            let _ = self.agent.update(&self.canister_id, "register_protocol")
                .with_arg(arg)
                .call_and_wait()
                .await?;
            Ok(())
        })
    }

    pub fn record_heartbeat(&self, subject: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.runtime.block_on(async {
            let time = Utc::now().to_rfc3339();
            let arg = Encode!(&subject, &time)?;
            let _ = self.agent.update(&self.canister_id, "record_heartbeat")
                .with_arg(arg)
                .call_and_wait()
                .await?;
            Ok(())
        })
    }
}
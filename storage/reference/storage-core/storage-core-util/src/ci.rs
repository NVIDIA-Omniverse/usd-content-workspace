// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::info_limited;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::Duration;

/// Port configuration structure
#[derive(Debug, Serialize, Deserialize)]
pub struct PortConfig {
    pub grpc_port: u16,
    pub rest_port: u16,
}

/// Serialize port configuration to a JSON file in the temp directory
/// The file is named after the current process PID
pub fn serialize_ports(
    grpc_port: u16,
    rest_port: u16,
    temp_dir: Option<PathBuf>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let config = PortConfig { grpc_port, rest_port };

    let file_path = get_file_path(process::id(), temp_dir);

    let json_content = serde_json::to_string_pretty(&config)?;
    fs::write(&file_path, json_content)?;

    info_limited!("Serialized ports {:?} to {}", config, file_path.display());

    Ok(file_path)
}

/// Read port configuration from a JSON file in the temp directory
/// Waits for the file to become available with retry logic
/// Deletes the file after successfully reading it
pub fn read_ports(pid: u32, temp_dir: Option<PathBuf>) -> Result<PortConfig, Box<dyn std::error::Error>> {
    let file_path = get_file_path(pid, temp_dir);

    // Retry loop to wait for file to become available
    let max_attempts = 30; // 30 seconds total
    let retry_delay = Duration::from_secs(1);

    for attempt in 1..=max_attempts {
        match fs::read_to_string(&file_path) {
            Ok(content) => {
                let config: PortConfig = serde_json::from_str(&content)?;

                // Delete the file after successfully reading it
                if let Err(e) = fs::remove_file(&file_path) {
                    info_limited!(
                        "Warning: Failed to delete port configuration file {}: {}",
                        file_path.display(),
                        e
                    );
                } else {
                    info_limited!("Deleted port configuration file {}", file_path.display());
                }

                return Ok(config);
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if attempt < max_attempts {
                    thread::sleep(retry_delay);
                    continue;
                } else {
                    return Err(format!("Port configuration file not found after {max_attempts} attempts: {e}").into());
                }
            },
            Err(e) => {
                return Err(format!("Error reading port configuration file: {e}").into());
            },
        }
    }

    Err("Failed to read port configuration file after maximum attempts".into())
}

pub fn get_file_path(pid: u32, temp_dir: Option<PathBuf>) -> PathBuf {
    let temp_dir = temp_dir.unwrap_or_else(std::env::temp_dir);
    temp_dir.join(format!("ports_{pid}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_serialize_and_read_ports() {
        // Create a temporary directory for testing
        let test_temp_dir = tempdir().unwrap();

        let grpc_port = 8080;
        let rest_port = 8081;

        // Test serialization with custom temp directory
        let file_path = serialize_ports(grpc_port, rest_port, Some(test_temp_dir.path().to_path_buf())).unwrap();
        assert!(file_path.exists());

        // Test reading
        let config = read_ports(process::id(), Some(test_temp_dir.path().to_path_buf())).unwrap();
        assert_eq!(config.grpc_port, grpc_port);
        assert_eq!(config.rest_port, rest_port);
        assert!(!file_path.exists());
    }
}

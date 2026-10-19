// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{fmt::Display, path::Path, sync::OnceLock, time::Duration};

use serde::{de::DeserializeOwned, Deserialize};

#[derive(Debug, Clone)]
pub struct Env {
    pub azure_service_bus: Option<AzureServiceBusEnv>,
    pub elasticmq: Option<ElasticMQEnv>,
    pub azure: Option<AzureBackendEnv>,
    pub s3: Option<S3BackendEnv>,
    pub s3_unversioned: Option<S3BackendEnv>,
    pub azure_table: Option<AzureTableBackendEnv>,
    pub dynamodb: Option<DynamoDBBackendEnv>,
    pub cosmosdb: Option<CosmosDBBackendEnv>,
    pub postgres: Option<PostgresBackendEnv>,
    pub redis: Option<RedisEnv>,
}

impl Env {
    #[track_caller]
    pub fn unwrap_azure_service_bus(&self) -> &AzureServiceBusEnv {
        self.azure_service_bus
            .as_ref()
            .expect("azure service bus env is missing")
    }

    #[track_caller]
    pub fn unwrap_elasticmq(&self) -> &ElasticMQEnv {
        self.elasticmq.as_ref().expect("elasticmq env is missing")
    }

    #[track_caller]
    pub fn unwrap_azure(&self) -> &AzureBackendEnv {
        self.azure.as_ref().expect("azure env is missing")
    }

    #[track_caller]
    pub fn unwrap_s3(&self) -> &S3BackendEnv {
        self.s3.as_ref().expect("s3 env is missing")
    }

    #[track_caller]
    pub fn unwrap_s3_unversioned(&self) -> &S3BackendEnv {
        self.s3_unversioned.as_ref().expect("s3 unversioned env is missing")
    }

    #[track_caller]
    pub fn unwrap_azure_table(&self) -> &AzureTableBackendEnv {
        self.azure_table.as_ref().expect("azure table env is missing")
    }

    #[track_caller]
    pub fn unwrap_dynamodb(&self) -> &DynamoDBBackendEnv {
        self.dynamodb.as_ref().expect("dynamodb env is missing")
    }

    #[track_caller]
    pub fn unwrap_cosmosdb(&self) -> &CosmosDBBackendEnv {
        self.cosmosdb.as_ref().expect("cosmosdb env is missing")
    }

    #[track_caller]
    pub fn unwrap_postgres(&self) -> &PostgresBackendEnv {
        self.postgres.as_ref().expect("postgres env is missing")
    }

    #[track_caller]
    pub fn unwrap_redis(&self) -> &RedisEnv {
        self.redis.as_ref().expect("redis env is missing")
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AzureServiceBusEnv {
    pub queue_namespace: String,
    pub queue_name: String,
    pub policy_name: String,
    pub policy_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ElasticMQEnv {
    pub host: String,
    pub port: u16,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub queue_url: String,
    pub region_name: String,
}

impl ElasticMQEnv {
    pub fn endpoint_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AzureBackendEnv {
    pub storage_account: String,
    pub storage_key: String,
    pub storage_container: String,
}

impl AzureBackendEnv {
    pub fn storage_account_url(&self) -> String {
        format!("https://{}.blob.core.windows.net/", self.storage_account)
    }

    pub fn storage_container_url(&self) -> String {
        format!(
            "https://{}.blob.core.windows.net/{}/",
            self.storage_account, self.storage_container
        )
    }

    pub fn resource_url(&self, resource: impl Display) -> String {
        format!(
            "https://{}.blob.core.windows.net/{}/{}",
            self.storage_account, self.storage_container, resource
        )
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct S3BackendEnv {
    pub host: String,
    pub port: u16,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl S3BackendEnv {
    pub fn bucket_url(&self) -> String {
        format!("http://{}:{}/{}/", self.host, self.port, self.bucket)
    }

    pub fn endpoint_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    pub fn resource_url(&self, resource: impl Display) -> String {
        format!("http://{}:{}/{}/{}", self.host, self.port, self.bucket, resource)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AzureTableBackendEnv {
    pub table_name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DynamoDBBackendEnv {
    pub host: String,
    pub port: u16,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region_name: String,
    pub table: String,
}

impl DynamoDBBackendEnv {
    pub fn endpoint_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CosmosDBBackendEnv {
    pub account: String,
    pub database_id: String,
    pub container_id: String,
    pub primary_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PostgresBackendEnv {
    pub user: String,
    pub password: String,
    pub dbname: String,
    pub host: String,
    pub port: u16,
}

impl PostgresBackendEnv {
    pub fn create_client(&self) -> postgres::Client {
        use postgres::{Client, NoTls};

        std::thread::spawn({
            let env = self.clone();
            move || {
                Client::configure()
                    .host(&env.host)
                    .port(env.port)
                    .user(&env.user)
                    .password(&env.password)
                    .dbname(&env.dbname)
                    .connect_timeout(Duration::from_secs(1))
                    .connect(NoTls)
                    .expect("failed to connect to postgres")
            }
        })
        .join()
        .unwrap()
    }

    pub fn create_schema(&self, schema: &str) {
        let query = format!("CREATE SCHEMA IF NOT EXISTS {schema}");
        let mut client = self.create_client();
        std::thread::spawn(move || client.execute(&query, &[]).expect("failed to create a schema"))
            .join()
            .unwrap();
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedisEnv {
    pub host: String,
    pub port: u16,
}

impl RedisEnv {
    pub fn address(&self) -> String {
        format!("redis://{}:{}", self.host, self.port)
    }
}

/// Get environment variables.
pub fn get_env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();

    ENV.get_or_init(|| Env {
        azure_service_bus: try_parse_env("AZURE_SERVICE_BUS_"),
        elasticmq: try_parse_env("ELASTICMQ_"),
        azure: try_parse_env("AZURE_BLOB_"),
        s3: try_parse_env("S3_"),
        s3_unversioned: try_parse_env("S3_UNVERSIONED_"),
        azure_table: try_parse_env("AZURE_TABLE_"),
        dynamodb: try_parse_env("DYNAMODB_"),
        cosmosdb: try_parse_env("COSMOSDB_"),
        postgres: try_parse_env("PG_"),
        redis: try_parse_env("REDIS_"),
    })
}

/// Get or create a test log file location.
pub fn get_logs_folder_path() -> &'static Path {
    use std::path::PathBuf;

    static LOGS_PATH: OnceLock<PathBuf> = OnceLock::new();

    LOGS_PATH.get_or_init(|| {
        // store test logs in the project root under the target directory, so its cleaned up with cargo clean
        let workspace_path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../target"))
            .canonicalize()
            .expect("failed to deduce a workspace path");
        let logs_path = workspace_path.join("test-logs");

        std::fs::create_dir_all(&logs_path).expect("failed to create logs dir");

        logs_path
    })
}

#[track_caller]
fn try_parse_env<T: DeserializeOwned>(prefix: &str) -> Option<T> {
    match envy::prefixed(prefix).from_env::<T>() {
        Ok(value) => Some(value),
        Err(error) => match error {
            envy::Error::MissingValue(_) => None,
            _ => panic!("env parse failed: {:?}", error),
        },
    }
}

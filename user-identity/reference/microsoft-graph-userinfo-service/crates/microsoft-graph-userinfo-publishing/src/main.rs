// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use clap::{Parser, Subcommand};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

#[derive(Parser)]
#[command(about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// OpenAPI, protos, licenses, and API reference HTML for NGC.
    Api {
        source: PathBuf,
        destination: PathBuf,
    },
    /// Service operations documentation HTML for NGC.
    ServiceDocs {
        source: PathBuf,
        destination: PathBuf,
    },
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Api {
            source,
            destination,
        } => pack_api_docs(&source, &destination),
        Command::ServiceDocs {
            source,
            destination,
        } => pack_service_docs(&source, &destination),
    }
}

fn pack_api_docs(src: &Path, dst: &Path) -> std::io::Result<()> {
    let dst_file = File::create(dst)?;

    let mut zip = zip::ZipWriter::new(dst_file);

    let openapi = src.join("openapi.yaml");
    add_file(&openapi, "openapi/userinfo/v1alpha/openapi.yaml", &mut zip)?;

    let license = src.join("LICENSE.txt");
    add_file(&license, "LICENSE.txt", &mut zip)?;

    let security = src.join("SECURITY.md");
    add_file(&security, "SECURITY.md", &mut zip)?;

    let protos = src.join("protos");
    add_folder(protos.to_str().unwrap(), "protos", &mut zip)?;

    let docs = src.join("docs/api/_build/docs/userinfo-api/latest");
    add_folder(docs.to_str().unwrap(), "docs", &mut zip)?;

    zip.finish()?;
    Ok(())
}

fn pack_service_docs(src: &Path, dst: &Path) -> std::io::Result<()> {
    let dst_file = File::create(dst)?;
    let mut zip = zip::ZipWriter::new(dst_file);

    let docs = src.join("docs/service/_build/docs/userinfo-service/latest");
    add_folder(docs.to_str().unwrap(), "docs", &mut zip)?;

    zip.finish()?;
    Ok(())
}

fn add_file(src: &PathBuf, dst: &str, zip: &mut zip::ZipWriter<File>) -> std::io::Result<()> {
    let zip_options: SimpleFileOptions = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    zip.start_file(dst, zip_options)?;

    let mut buffer = Vec::new();
    let mut file = File::open(src)?;
    file.read_to_end(&mut buffer)?;

    zip.write_all(&buffer)?;
    buffer.clear();

    Ok(())
}

fn add_folder(src: &str, dst: &str, zip: &mut zip::ZipWriter<File>) -> std::io::Result<()> {
    let zip_options: SimpleFileOptions = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let walkdir = WalkDir::new(src);

    let src_path = Path::new(src);
    let dst_path = Path::new(dst);

    for entry in walkdir {
        let entry = entry?;
        let entry_path = entry.path();
        let path_prefixed = entry_path.strip_prefix(src_path).unwrap();
        let path = path_prefixed.to_str().unwrap();
        let entry_dst = if path.is_empty() {
            dst_path.to_path_buf()
        } else {
            dst_path.join(path)
        };
        let entry_dst = entry_dst.to_str().unwrap();

        if entry_path.is_file() {
            let entry_path_buf = entry_path.to_path_buf();
            add_file(&entry_path_buf, entry_dst, zip)?;
        } else if entry_path.is_dir() {
            zip.add_directory(entry_dst, zip_options)?;
        }
    }
    Ok(())
}

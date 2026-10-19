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

use std::fmt::Display;

use serde_json::{json, Value as JsonValue};
use storage_service_protos::{
    google::protobuf::{value::Kind, Struct, Timestamp, Value},
    nvidia::omniverse::notifications::publisher::v1beta::{Event, EventResource},
};
use time::{macros::format_description, OffsetDateTime};

pub fn create_object_created_event(
    address: impl Into<String>,
    identity: impl Into<String>,
    sequencer: impl Into<String>,
    time: OffsetDateTime,
) -> Event {
    let address = address.into();
    Event {
        event_type: "omni.storage.created".into(),
        occurred_at: Some(create_timestamp(time)),
        message: Some(Struct {
            fields: [
                create_string_field("resource_address", &address),
                create_string_field("resource_identity", identity),
                create_string_field("sequencer", sequencer),
            ]
            .into(),
        }),
        resource: Some(EventResource { resource_id: address }),
    }
}

pub fn create_object_deleted_event(
    address: impl Into<String>,
    sequencer: impl Into<String>,
    time: OffsetDateTime,
) -> Event {
    let address = address.into();
    Event {
        event_type: "omni.storage.deleted".into(),
        occurred_at: Some(create_timestamp(time)),
        message: Some(Struct {
            fields: [
                create_string_field("resource_address", &address),
                create_string_field("sequencer", sequencer),
            ]
            .into(),
        }),
        resource: Some(EventResource { resource_id: address }),
    }
}

/// Build the `Event` shape the notification service emits for a synthetic dir_created /
/// dir_deleted event. `event_type` selects `"omni.storage.dir_created"` /
/// `"omni.storage.dir_deleted"`; the message payload carries only `resource_address`.
pub fn create_dir_event(event_type: impl Into<String>, address: impl Into<String>, time: OffsetDateTime) -> Event {
    let address = address.into();
    Event {
        event_type: event_type.into(),
        occurred_at: Some(create_timestamp(time)),
        message: Some(Struct {
            fields: [create_string_field("resource_address", &address)].into(),
        }),
        resource: Some(EventResource { resource_id: address }),
    }
}

/// Create an S3 event message.
pub fn create_s3_event(records: impl IntoIterator<Item = JsonValue>) -> String {
    json!({ "Records": records.into_iter().collect::<Vec<_>>() }).to_string()
}

/// Create an S3 event record.
#[allow(clippy::too_many_arguments)]
pub fn create_s3_event_record(
    name: impl Into<String>,
    bucket: impl Into<String>,
    region: impl Into<String>,
    key: impl Into<String>,
    version_id: impl Into<String>,
    etag: impl Into<String>,
    sequencer: impl Into<String>,
    time: OffsetDateTime,
) -> JsonValue {
    let event_time = format_s3_time(time);
    json!({
        "eventVersion": "2.1",
        "eventSource": "aws:s3",
        "awsRegion": region.into(),
        "eventTime": event_time,
        "eventName": name.into(),
        "userIdentity": {
            "principalId": "AWS:OIDWBE8942N:myprincipal"
        },
        "requestParameters": {
            "sourceIPAddress": "1.1.1.1"
        },
        "responseElements": {
            "x-amz-request-id": "DFON3456NQ43Y7U",
            "x-amz-id-2": "88ZlQDPmJOojAiTg01fOAR1iV5qoMXl+um1LRmNklVGWFvJwNyn5W5E4Q7+yJ7azsa5/HT4QEx8Z4/rSb0I4UkA4WO1A+/mC"
        },
        "s3": {
            "s3SchemaVersion": "1.0",
            "configurationId": "mytestbucketevents",
            "bucket": {
                "name": bucket.into(),
                "ownerIdentity": {
                    "principalId": "EDFWWON3011"
                },
                "arn": "arn:aws:s3:::mytest"
            },
            "object": {
                "key": key.into(),
                "size": 22,
                "eTag": etag.into(),
                "versionId": version_id.into(),
                "sequencer": sequencer.into(),
            }
        }
    })
}

/// Create a BlobCreated event.
pub fn create_azure_blob_created_event(
    account: impl Display,
    container: impl Display,
    blob: impl Display,
    etag: impl Into<String>,
    sequencer: impl Into<String>,
    time: OffsetDateTime,
) -> String {
    create_azure_event(create_azure_blob_event(
        "Microsoft.Storage.BlobCreated",
        account,
        container,
        blob,
        etag,
        sequencer,
        time,
    ))
}

/// Create an Azure event message.
pub fn create_azure_event(event: JsonValue) -> String {
    event.to_string()
}

/// Create an Azure Blob event.
#[allow(clippy::too_many_arguments)]
pub fn create_azure_blob_event(
    event_type: impl Into<String>,
    account: impl Display,
    container: impl Display,
    blob: impl Display,
    etag: impl Into<String>,
    sequencer: impl Into<String>,
    time: OffsetDateTime,
) -> JsonValue {
    json!({
        "id": "98e5fb8f-601e-00a5-7aea-efb71706d71b",
        "source": format!("/subscriptions/mysubscription/resourceGroups/DefaultResourceGroup-EUS/providers/Microsoft.Storage/storageAccounts/{}", account),
        "specversion": "1.0",
        "type": event_type.into(),
        "subject": format!("/blobServices/default/containers/{}/blobs/{}", container, blob),
        "time": format_azure_blob_time(time),
        "data":{
            "api": "PutBlob",
            "clientRequestId": "e8a62567-64de-4422-a216-f4efcca3a012",
            "requestId": "98e5fb8f-601e-00a5-7aea-efb717000000",
            "contentType": "application/x-zip-compressed",
            "contentLength": 22,
            "blobType": "BlockBlob",
            "accessTier": "Default",
            "url": format!("https://{}.blob.core.windows.net/{}/{}", account, container, blob),
            "eTag": etag.into(),
            "sequencer": sequencer.into(),
            "storageDiagnostics": {"batchId": "f6de2ca3-6006-006e-00ea-efb442000000"}
        }
    })
}

/// Create a BlobDeleted event.
pub fn create_azure_blob_deleted_event(
    account: impl Display,
    container: impl Display,
    blob: impl Display,
    sequencer: impl Into<String>,
    time: OffsetDateTime,
) -> String {
    create_azure_event(create_azure_blob_event(
        "Microsoft.Storage.BlobDeleted",
        account,
        container,
        blob,
        "0x8DDBE01246030B0",
        sequencer,
        time,
    ))
}

fn format_s3_time(time: OffsetDateTime) -> String {
    time.to_utc()
        .format(&format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .unwrap()
}

fn format_azure_blob_time(time: OffsetDateTime) -> String {
    time.to_utc()
        .format(&format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:7]Z"
        ))
        .unwrap()
}

fn create_string_field(key: impl Into<String>, value: impl Into<String>) -> (String, Value) {
    (
        key.into(),
        Value {
            kind: Some(Kind::StringValue(value.into())),
        },
    )
}

fn create_timestamp(time: OffsetDateTime) -> Timestamp {
    let time_utc = time.to_utc();
    Timestamp {
        seconds: time_utc.unix_timestamp(),
        nanos: time_utc.nanosecond() as i32,
    }
}
